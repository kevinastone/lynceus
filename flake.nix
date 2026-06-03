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

          image =
            with pkgs;
            dockerTools.buildImage {
              name = default.pname;
              copyToRoot = with dockerTools; [
                usrBinEnv
                binSh
                coreutils
                caCertificates
                fakeNss
              ];
              config.Entrypoint = [ "${lynceus}/bin/lynceus" ];
              config.Labels = with cargoToml; {
                "org.opencontainers.image.title" = package.name;
                "org.opencontainers.image.source" = package.repository or "";
                "org.opencontainers.image.description" = package.description or "";
              };
            };

          inherit (pkgs) skopeo;
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
              skopeo
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
