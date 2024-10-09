{ pkgs }: 
{
  default = pkgs.callPackage ./nix/package.nix { };
  local = pkgs.callPackage ./nix/package-from-source.nix { };
  nightly = pkgs.callPackage ./nix/package-nightly.nix { };
}
