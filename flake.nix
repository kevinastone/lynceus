{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "github:nix-community/naersk/master";
      inputs.fenix.follows = "fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      nixpkgs,
      utils,
      naersk,
      fenix,
      treefmt-nix,
      ...
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        inherit (fenix.packages.${system}) stable;
        toolchain = fenix.packages.${system}.combine [
          stable.cargo
          stable.rustc
          stable.rustfmt
          stable.clippy
        ];
        lib = pkgs.callPackage naersk {
          cargo = toolchain;
          rustc = toolchain;
        };
        treefmtStack = treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";
          programs.rustfmt = {
            enable = true;
            package = toolchain;
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
        commonArgs = {
          src = ./.;
          buildInputs = with pkgs; [ openssl ];
          nativeBuildInputs = with pkgs; [ pkg-config ];
          CARGO_HTTP_USER_AGENT = "kstone/argus (://github.com/kstone/argus)";
        };
      in
      {
        packages = rec {
          argus = lib.buildPackage commonArgs;
          bin = argus;
          default = argus;

          check = lib.buildPackage (
            commonArgs
            // {
              mode = "check";
              release = false;
            }
          );
          clippy = lib.buildPackage (
            commonArgs
            // {
              mode = "clippy";
              release = false;
            }
          );
          test = lib.buildPackage (
            commonArgs
            // {
              mode = "test";
              release = false;
            }
          );

          image =
            with pkgs;
            dockerTools.buildImage {
              name = "argus";
              copyToRoot = buildEnv {
                name = "image-root";
                paths = [ cacert ];
              };
              config.Entrypoint = [ "${argus}/bin/argus" ];
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
            nativeBuildInputs = [ toolchain ] ++ commonArgs.nativeBuildInputs;
            inherit (commonArgs) buildInputs CARGO_HTTP_USER_AGENT;
            packages = [
              skopeo
              cargo-outdated
            ];
            RUST_SRC_PATH = rustPlatform.rustLibSrc;
          };
      }
    );
}
