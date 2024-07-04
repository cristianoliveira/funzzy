{
  description = "Funzzy (fzz) - the lightweight watcher";

  inputs.nixpkgs.url = "github:nixos/nixpkgs";

  outputs = { self, nixpkgs, ... }:
    let
      lib = nixpkgs.lib;
      recursiveMergeAttrs = listOfAttrsets: lib.fold (attrset: acc: lib.recursiveUpdate attrset acc) {} listOfAttrsets;
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      systemPackages = map (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          packages."${system}".funzzy = pkgs.callPackage ./nix/package.nix {};

          devShells."${system}".default = pkgs.mkShell {
            packages = with pkgs; [
              rustc
              cargo
              rustfmt

              libiconv

              # if system contains "darwin" then darwin.apple_sdk.frameworks.CoreServices else null
              # Fix error: `ld: framework not found CoreServices`
              (if system == "x86_64-darwin" || 
                  system == "aarch64-darwin" 
                then darwin.apple_sdk.frameworks.CoreServices
                else null
              )
            ];
          };
        }
      ) systems;
    in
      # Reduce the list of packages of packages into a single attribute set
      recursiveMergeAttrs(systemPackages);
}
