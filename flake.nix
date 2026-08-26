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
        srcpkgs = import ./default.nix { inherit pkgs; };

        # Cross-compiled aarch64-linux funzzy from any host (used by the
        # release workflow on x86_64-linux runners).
        # Emulated builds (qemu-user + `--system aarch64-linux`) are broken:
        # nixpkgs' fetch-cargo-vendor-util resolves its python interpreter
        # through PATH at runtime and picks a requests-less env in the
        # emulated sandbox (`ModuleNotFoundError: No module named 'requests'`).
        # Cross builds run every build-time tool natively; only rust targets
        # aarch64. Cross-compiled binaries cannot run on the host, so tests
        # stay on native systems (CI integration + agent-final watcher gate).
        local-aarch64 = nixpkgs.legacyPackages.${system}.pkgsCross.aarch64-multiplatform
          .callPackage ./nix/package-local.nix { doCheck = false; };
      in {
        packages = srcpkgs // { inherit local-aarch64; };

        devShells.default = pkgs.callPackage ./shell.nix { inherit pkgs srcpkgs; };
    });
}
