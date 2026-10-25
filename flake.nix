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
          dockerTools.buildLayeredImage {
            name = cargoToml.package.name;
            contents = with dockerTools; [
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

        crossTargets = {
          amd64 = "x86_64-unknown-linux-musl";
          arm64 = "aarch64-unknown-linux-musl";
          amd64-glibc = "x86_64-linux";
          arm64-glibc = "aarch64-linux";
        };

        makeMultiArchImage =
          images:
          with pkgs;
          with lib;
          let
            ociDir =
              runCommand "${cargoToml.package.name}-multiarch-oci"
                {
                  nativeBuildInputs = [ regctl ];
                  passthru.push = writeShellApplication {
                    name = "push-multiarch-oci";
                    runtimeInputs = [ regctl ];
                    text = ''
                      for dest in "$@"; do
                        echo "Pushing multi-arch image to $dest..."
                        regctl image copy "ocidir://${ociDir}" "$dest"
                      done
                    '';
                  };
                }
                (
                  concatLines (
                    mapAttrsToList (arch: img: "regctl image import ocidir://image:${arch} ${img}") images
                    ++ [
                      "regctl index create ocidir://image ${
                        concatMapStringsSep " " (arch: "--ref ocidir://image:${arch} --platform linux/${arch}") (
                          attrNames images
                        )
                      }"
                    ]
                    ++ mapAttrsToList (arch: _: "regctl tag delete ocidir://image:${arch}") images
                    ++ [ ''mv image "$out"'' ]
                  )
                );
          in
          ociDir;

        crossBins = pkgs.lib.mapAttrs' (
          name: target: pkgs.lib.nameValuePair "bin-${name}" (makeBinary target)
        ) crossTargets;

        crossImages = pkgs.lib.mapAttrs' (
          name: _: pkgs.lib.nameValuePair "image-${name}" (makeImage crossBins."bin-${name}")
        ) crossTargets;
      in
      rec {
        packages = rec {
          bin = makeBinary system;
          default = bin;

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

          image = makeImage bin;
          image-multiarch = makeMultiArchImage {
            amd64 = crossImages.image-amd64;
            arm64 = crossImages.image-arm64;
          };
          image-multiarch-glibc = makeMultiArchImage {
            amd64 = crossImages.image-amd64-glibc;
            arm64 = crossImages.image-arm64-glibc;
          };
        }
        // crossBins
        // crossImages;

        checks = {
          build = packages.bin;
          inherit (packages) clippy test;
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

            RUST_SRC_PATH = "${rustPlatform.rustLibSrc}";
          };
      }
    );
}
