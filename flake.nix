{
  description = "GitLab To-Do Helper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      imports = [];

      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];

      perSystem = {
        pkgs,
        inputs',
        system,
        ...
      }: {
        _module.args.pkgs = import inputs.nixpkgs {
          inherit system;
          overlays = [inputs.rust-overlay.overlays.default];
        };

        devShells.default = pkgs.mkShell {
          name = "dev";
          packages = with pkgs;
            [
              rust-bin.stable.latest.default
              rust-analyzer
            ]
            ++ lib.optionals stdenv.isDarwin (with darwin.apple_sdk.frameworks; [
              Security
            ]);
        };
      };
    };
}
