{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      utils,
      treefmt-nix,
      crane,
      ...
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
        craneLib = crane.mkLib pkgs;
        treefmtStack = treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";
          programs.rustfmt = {
            enable = true;
            edition = "2024";
          };
          # Nix formatters
          programs.nixfmt.enable = true;
          programs.statix.enable = true;
          programs.deadnix.enable = true;
          settings.formatter = {
            deadnix.priority = 1;
            statix.priority = 2;
            nixfmt.priority = 3;
          };
        };

        src = craneLib.cleanCargoSource (craneLib.path ./.);

        commonArgs = {
          inherit src;
          strictDeps = true;
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
        };

        # Build only dependencies to cache them
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the final binary using cached dependency artifacts
        lynceus = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );

        # Helper to compile the container image for Linux (either natively or cross-compiled)
        makeImage =
          targetSystem:
          let
            targetPkgs =
              if targetSystem == system then
                pkgs
              else
                import nixpkgs {
                  inherit system;
                  crossSystem = targetSystem;
                };

            targetCraneLib = crane.mkLib targetPkgs;
            isCross = targetPkgs.stdenv.hostPlatform != targetPkgs.stdenv.buildPlatform;

            crossArgs =
              commonArgs
              // (targetPkgs.lib.optionalAttrs isCross (
                let
                  rustHostTriple = targetPkgs.stdenv.buildPlatform.config;
                  rustHostTripleEnv = builtins.replaceStrings [ "-" ] [ "_" ] rustHostTriple;
                  nixHostSuffix =
                    if targetPkgs.stdenv.buildPlatform.isDarwin && targetPkgs.stdenv.buildPlatform.isAarch64 then
                      "arm64_apple_darwin"
                    else
                      rustHostTripleEnv;
                  cargoHostLinkerVar = "CARGO_TARGET_" + targetPkgs.lib.toUpper rustHostTripleEnv + "_LINKER";
                  isDarwin = targetPkgs.stdenv.buildPlatform.isDarwin;
                in
                {
                  HOST_CC = "${targetPkgs.buildPackages.stdenv.cc}/bin/cc";
                  HOST_CXX = "${targetPkgs.buildPackages.stdenv.cc}/bin/c++";
                  "${cargoHostLinkerVar}" = "${targetPkgs.buildPackages.stdenv.cc}/bin/cc";
                }
                // targetPkgs.lib.optionalAttrs isDarwin {
                  depsBuildBuild = [
                    targetPkgs.libiconv
                  ];
                  nativeBuildInputs = [
                    targetPkgs.buildPackages.libiconv
                  ];
                  "NIX_LDFLAGS_${nixHostSuffix}" = "-L${targetPkgs.buildPackages.libiconv}/lib";
                  "NIX_CFLAGS_COMPILE_${nixHostSuffix}" = "-I${targetPkgs.buildPackages.libiconv}/include";
                }
              ));

            cargoArtifactsCross = targetCraneLib.buildDepsOnly crossArgs;

            lynceusCross = targetCraneLib.buildPackage (
              crossArgs
              // {
                inherit cargoArtifactsCross;
              }
            );
          in
          with targetPkgs;
          dockerTools.buildImage {
            name = cargoToml.package.name;
            copyToRoot = with dockerTools; [
              usrBinEnv
              binSh
              coreutils
              caCertificates
              fakeNss
            ];
            config.Entrypoint = [ "${lynceusCross}/bin/lynceus" ];
            config.Labels = with cargoToml; {
              "org.opencontainers.image.title" = package.name;
              "org.opencontainers.image.source" = package.repository or "";
              "org.opencontainers.image.description" = package.description or "";
            };
          };

        push-multiarch = pkgs.writeShellApplication {
          name = "push-multiarch";
          runtimeInputs = with pkgs; [
            regctl
            gzip
            coreutils
          ];
          text = ''
            if [ "$#" -lt 3 ]; then
              echo "Usage: push-multiarch <registry-repo> <amd64-image-tar> <arm64-image-tar>"
              exit 1
            fi

            REPO=$(echo "$1" | tr '[:upper:]' '[:lower:]')
            AMD64_IMAGE="$2"
            ARM64_IMAGE="$3"

            if [ -z "''${TAGS:-}" ]; then
              echo "Error: TAGS environment variable is not set"
              exit 1
            fi


            # Import images into local OCI layout directories directly from Nix build outputs
            regctl image import ocidir://./local-oci-amd64 "$AMD64_IMAGE"
            regctl image import ocidir://./local-oci-arm64 "$ARM64_IMAGE"

            # Get the digests of the imported OCI layouts
            AMD64_DIGEST=$(regctl image digest ocidir://./local-oci-amd64)
            ARM64_DIGEST=$(regctl image digest ocidir://./local-oci-arm64)

            # Push single-architecture layers and manifests by digest
            echo "Pushing AMD64 digest: $AMD64_DIGEST to $REPO..."
            regctl image copy ocidir://./local-oci-amd64 "$REPO@$AMD64_DIGEST"

            echo "Pushing ARM64 digest: $ARM64_DIGEST to $REPO..."
            regctl image copy ocidir://./local-oci-arm64 "$REPO@$ARM64_DIGEST"

            # Create and push the multi-architecture manifest index for each tag
            # Since TAGS is multiline, we read it line by line
            echo "$TAGS" | while read -r tag || [ -n "$tag" ]; do
              if [ -n "$tag" ]; then
                echo "Creating and pushing multi-arch index for $tag..."
                regctl index create "$tag" \
                  --ref "$REPO@$AMD64_DIGEST" \
                  --platform linux/amd64 \
                  --ref "$REPO@$ARM64_DIGEST" \
                  --platform linux/arm64
              fi
            done

            # Cleanup local OCI layouts
            rm -rf ./local-oci-amd64 ./local-oci-arm64
          '';
        };
      in
      rec {
        packages = rec {
          inherit lynceus;
          bin = lynceus;
          default = lynceus;

          check = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "lynceus-check";
              cargoBuildCommand = "cargo check";
            }
          );

          clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          test = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
            }
          );

          image = makeImage system;
          image-amd64 = makeImage "x86_64-linux";
          image-arm64 = makeImage "aarch64-linux";

          inherit push-multiarch;
        };

        checks = {
          inherit (packages) check clippy test;
          formatting = treefmtStack.config.build.check self;
        };

        formatter = treefmtStack.config.build.wrapper;
        devShells.default =
          with pkgs;
          craneLib.devShell {
            checks = self.checks.${system};
            packages = [
              cargo-outdated
              cargo-release
              git-cliff
            ];

            # Extract the Rust standard library source dynamically from craneLib's toolchain
            RUST_SRC_PATH = craneLib.callPackage (
              { rustc, rustPlatform }:
              if builtins.pathExists "${rustc}/lib/rustlib/src/rust/library" then
                "${rustc}/lib/rustlib/src/rust/library"
              else
                rustPlatform.rustLibSrc
            ) { };
          };
      }
    );
}
