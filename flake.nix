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
          doCheck = false;
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
        }
        // craneLib.crateNameFromCargoToml { inherit src; };

        # Build only dependencies to cache them
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        getTargetPkgs =
          targetSystem:
          if targetSystem == system then
            pkgs
          else
            import nixpkgs {
              inherit system;
              crossSystem = targetSystem;
            };

        # Helper to compile the binary for a target system (either natively or cross-compiled)
        makeBinary =
          targetSystem:
          let
            targetPkgs = getTargetPkgs targetSystem;

            inherit (targetPkgs) stdenv lib;
            targetCraneLib = crane.mkLib targetPkgs;
            isCross = stdenv.hostPlatform != stdenv.buildPlatform;
            isBuildDarwin = stdenv.buildPlatform.isDarwin;

            args =
              commonArgs
              // lib.optionalAttrs (isCross && isBuildDarwin) {
                depsBuildBuild = [
                  targetPkgs.libiconv
                ];
                "NIX_LDFLAGS" = "-L${targetPkgs.buildPackages.libiconv}/lib";
              };

            cargoArtifacts = targetCraneLib.buildDepsOnly args;
          in
          targetCraneLib.buildPackage (
            args
            // {
              inherit cargoArtifacts;
              passthru = {
                inherit targetSystem;
              };
              meta.mainProgram = args.pname;
            }
          );

        # Helper to compile the container image for Linux using a pre-built crossSystem package
        makeImage =
          bin:
          let
            inherit (bin) targetSystem;
            targetPkgs = getTargetPkgs targetSystem;
          in
          with targetPkgs;
          dockerTools.buildImage {
            name = cargoToml.package.name;
            copyToRoot = with dockerTools; [
              caCertificates
              fakeNss
            ];
            config.Entrypoint = [ (lib.getExe bin) ];
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
          bin = makeBinary system;
          default = bin;

          build = craneLib.cargoBuild (
            commonArgs
            // {
              inherit cargoArtifacts;
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
              doCheck = true;
              inherit cargoArtifacts;
            }
          );

          bin-amd64 = makeBinary "x86_64-unknown-linux-musl";
          bin-arm64 = makeBinary "aarch64-unknown-linux-musl";
          bin-amd64-glibc = makeBinary "x86_64-linux";
          bin-arm64-glibc = makeBinary "aarch64-linux";

          image = makeImage bin;
          image-amd64 = makeImage bin-amd64;
          image-arm64 = makeImage bin-arm64;
          image-amd64-glibc = makeImage bin-amd64-glibc;
          image-arm64-glibc = makeImage bin-arm64-glibc;

          inherit push-multiarch;
        };

        checks = {
          inherit (packages) build clippy test;
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
