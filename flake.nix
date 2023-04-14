{
  description = "GitLab To-Do Helper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];

      perSystem = {pkgs, ...}: {
        devShells.default = pkgs.mkShell {
          name = "dev";
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            rust-analyzer
            libiconv
          ];
        };
      };
    };
}
