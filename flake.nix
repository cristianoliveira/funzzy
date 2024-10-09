{
  description = "Funzzy (fzz) - the lightweight blazingly fast watcher";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils, ... }: 
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        srcpkgs = import ./packages.nix { inherit pkgs; };
      in {
        packages = srcpkgs;

        devShells.default = pkgs.callPackage ./shell.nix { inherit pkgs srcpkgs; };
    });
}
