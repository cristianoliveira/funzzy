{
  description = "Funzzy (fzz) - the lightweight blazingly fast watcher";

  inputs.nixpkgs.url = "github:nixos/nixpkgs";

  outputs = { self, nixpkgs, ... }:
    let
      lib = nixpkgs.lib;
      recursiveMergeAttrs = listOfAttrsets: lib.fold (attrset: acc: lib.recursiveUpdate attrset acc) {} listOfAttrsets;
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      systemPackages = map (system:
        let
          pkgs = import nixpkgs { 
            inherit system;
            overlays = [ 
              (final: prev: {
                  copkgs = {
                    funzzy = prev.callPackage ./nix/package.nix {};
                    funzzyNightly = prev.callPackage ./nix/package-from-source.nix {};
                  };
                }
              )
            ];
          };
        in
        {
          packages."${system}" = {
            funzzy = pkgs.copkgs.funzzy;
            funzzyNightly = pkgs.copkgs.funzzyNightly;
          };

          devShells."${system}".default = import ./nix/development-environment.nix { inherit pkgs; };
        }
      ) systems;
    in
      # Reduce the list of packages of packages into a single attribute set
      recursiveMergeAttrs(systemPackages);
}
