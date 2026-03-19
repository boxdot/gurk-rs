{
  description = "gurk - Signal Messenger client for terminal";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    supportedSystems = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
  in {
    packages = forAllSystems (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        gurk = pkgs.rustPlatform.buildRustPackage {
          pname = "gurk";
          version = "0.9.0-dev";

          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.intersection (pkgs.lib.fileset.fromSource (pkgs.lib.sources.cleanSource ./.)) (
              pkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./migrations
                ./.sqlx
                ./xtask
                ./benches
              ]
            );
          };

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "curve25519-dalek-4.1.3" = "sha256-bPh7eEgcZnq9C3wmSnnYv0C4aAP+7pnwk9Io29GrI4A=";
              "libsignal-protocol-0.1.0" = "sha256-k0V5wnHGUwE76mZlG/2kWRWVgpvTgGDt9JuxdsoJZ7Y=";
              "libsignal-service-0.1.0" = "sha256-rnk+YCFZ1eGANrsHEq3mcaNrpUbKoVBGLYNeQG8EuXc=";
              "presage-0.8.0-dev" = "sha256-OnbJCrDrGMDzwHfi0ygxa3ZAiZ2KnJ0Ux638O6O0GZo=";
              "libsqlite3-sys-0.36.0" = "sha256-JK9U4zIveudVArQQS3roHXsUWGFVcI43uWVcT8++l2Q=";
              "spqr-1.4.0" = "sha256-2jZGm9ePg431jvjqBXU7d1NJ00TPIKtwsfdspme1okM=";
              "sqlx-0.8.6" = "sha256-miIFu+YavrRPbVXS0+OE3y3+85zZ5dsozG6M+Qb06fc=";
            };
          };

          nativeBuildInputs = with pkgs; [
            protobuf
            pkg-config
          ];

          buildInputs = with pkgs; [
            openssl
          ];

          # Use system OpenSSL instead of vendoring it.
          # libsqlite3-sys still bundles SQLCipher with its own OpenSSL via
          # the bundled-sqlcipher-vendored-openssl cargo feature.
          OPENSSL_NO_VENDOR = "1";
          PROTOC = "${pkgs.protobuf}/bin/protoc";

          # The .cargo/config.toml contains cross-compilation settings
          # (custom linkers, target-specific env vars) that conflict with
          # the nix build environment.
          postPatch = ''
            rm -f .cargo/config.toml
          '';

          preCheck = ''
            export HOME=$(mktemp -d)
          '';

          meta = {
            description = "Signal Messenger client for terminal";
            homepage = "https://github.com/boxdot/gurk-rs";
            license = pkgs.lib.licenses.agpl3Only;
            mainProgram = "gurk";
          };
        };
      in {
        inherit gurk;
        default = gurk;
      }
    );

    devShells = forAllSystems (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        default = pkgs.mkShell {
          inputsFrom = [self.packages.${system}.gurk];

          packages = with pkgs; [
            cargo
            rustc
            rust-analyzer
            clippy
            rustfmt
          ];

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      }
    );

    overlays.default = final: _prev: {
      gurk = self.packages.${final.system}.gurk;
    };
  };
}
