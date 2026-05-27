{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      nixpkgs,
      utils,
      treefmt-nix,
      ...
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
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

        cargoToml = fromTOML (builtins.readFile ./Cargo.toml);

        commonArgs = {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = ./.;
          cargoHash = "sha256-WwVHqxC/p6TMiVa48LODvok3UEfM8vAUaBAGWrLOCls=";
          buildInputs = with pkgs; [ openssl ];
          nativeBuildInputs = with pkgs; [ pkg-config ];
        };

        argus = pkgs.rustPlatform.buildRustPackage commonArgs;
      in
      {
        packages = rec {
          inherit argus;
          bin = argus;
          default = argus;

          check = pkgs.rustPlatform.buildRustPackage (
            commonArgs
            // {
              pname = "argus-check";
              buildPhase = "cargo check";
              installPhase = "touch $out";
            }
          );

          clippy = pkgs.rustPlatform.buildRustPackage (
            commonArgs
            // {
              pname = "argus-clippy";
              nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.clippy ];
              buildPhase = "cargo clippy -- -D warnings";
              installPhase = "touch $out";
            }
          );

          test = pkgs.rustPlatform.buildRustPackage (
            commonArgs
            // {
              pname = "argus-test";
              buildPhase = "cargo test";
              installPhase = "touch $out";
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
                "org.opencontainers.image.source" = "https://github.com/kstone/argus";
                "org.opencontainers.image.description" = ''
                  argus is a file watcher that reports file changes using a webhook.
                '';
              };
            };

          inherit (pkgs) skopeo;
        };

        formatter = treefmtStack.config.build.wrapper;
        devShells.default =
          with pkgs;
          mkShell {
            nativeBuildInputs = [
              rustc
              cargo
              rustfmt
              clippy
            ]
            ++ commonArgs.nativeBuildInputs;
            inherit (commonArgs) buildInputs;
            packages = [
              skopeo
              cargo-outdated
            ];
            RUST_SRC_PATH = rustPlatform.rustLibSrc;
          };
      }
    );
}
