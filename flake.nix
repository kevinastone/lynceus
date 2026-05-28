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
          buildInputs = with pkgs; [ openssl ];
          nativeBuildInputs = with pkgs; [ pkg-config ];
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
        };

        # Build only dependencies to cache them
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the final binary using cached dependency artifacts
        argus = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
          }
        );
      in
      rec {
        packages = rec {
          inherit argus;
          bin = argus;
          default = argus;

          check = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "argus-check";
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
              name = "argus";
              copyToRoot = buildEnv {
                name = "image-root";
                paths = [
                  cacert
                  argus
                  bashInteractive
                  coreutils
                ];
                pathsToLink = [
                  "/bin"
                  "/etc"
                ];
              };
              config.Env = [
                "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
              ];
              config.Entrypoint = [ "/bin/argus" ];
              config.Labels = {
                "org.opencontainers.image.title" = "argus";
                "org.opencontainers.image.source" = "https://github.com/kevinastone/argus";
                "org.opencontainers.image.description" = ''
                  argus is a file watcher that reports file changes using a webhook.
                '';
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
            ];
          };
      }
    );
}
