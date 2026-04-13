{
  description = "forge: AI enabled pair programmer for Claude, GPT, O Series, Grok, Deepseek, Gemini and 300+ models";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      formatter = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        pkgs.nixfmt-rfc-style
      );

      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
          src = lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              lib.cleanSourceFilter path type
              && baseNameOf path != "target"
              && baseNameOf path != "result";
          };
          forge = pkgs.rustPlatform.buildRustPackage {
            pname = "forge";
            version = "0.1.0-dev";
            inherit src;

            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };

            cargoBuildFlags = [
              "-p"
              "forge_main"
              "--bin"
              "forge"
            ];
            cargoInstallFlags = [
              "-p"
              "forge_main"
              "--bin"
              "forge"
            ];

            nativeBuildInputs = [
              pkgs.cmake
              pkgs.nasm
              pkgs.perl
              pkgs.pkg-config
              pkgs.protobuf
            ];

            buildInputs =
              [ pkgs.sqlite ]
              ++ lib.optionals pkgs.stdenv.isLinux [
                pkgs.libxkbcommon
                pkgs.libx11
                pkgs.libxext
                pkgs.libxfixes
                pkgs.libxcb
                pkgs.wayland
              ]
              ++ lib.optionals pkgs.stdenv.isDarwin [
                pkgs.libiconv
                pkgs.apple-sdk
              ];

            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";
            APP_VERSION = "0.1.0-dev";

            doCheck = false;

            meta = {
              description = "forge: AI enabled pair programmer for Claude, GPT, O Series, Grok, Deepseek, Gemini and 300+ models";
              homepage = "https://forgecode.dev";
              license = lib.licenses.mit;
              mainProgram = "forge";
              platforms = lib.platforms.unix;
            };
          };
        in
        {
          default = forge;
          forge = forge;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/forge";
        };
        forge = {
          type = "app";
          program = "${self.packages.${system}.forge}/bin/forge";
        };
      });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
        in
        {
          default = pkgs.mkShell {
            packages =
              [
                pkgs.cargo
                pkgs.cargo-insta
                pkgs.cargo-llvm-cov
                pkgs.clippy
                pkgs.cmake
                pkgs.nasm
                pkgs.perl
                pkgs.pkg-config
                pkgs.protobuf
                pkgs.rust-analyzer
                pkgs.rustc
                pkgs.rustfmt
                pkgs.sqlite
              ]
              ++ lib.optionals pkgs.stdenv.isLinux [
                pkgs.libxkbcommon
                pkgs.libx11
                pkgs.libxext
                pkgs.libxfixes
                pkgs.libxcb
                pkgs.wayland
              ]
              ++ lib.optionals pkgs.stdenv.isDarwin [
                pkgs.libiconv
                pkgs.darwin.apple_sdk.frameworks.AppKit
                pkgs.darwin.apple_sdk.frameworks.CoreGraphics
                pkgs.darwin.apple_sdk.frameworks.Foundation
              ];

            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";
            APP_VERSION = "0.1.0-dev";
          };
        });
    };
}
