{ lib , rustPlatform , fetchFromGitHub }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "2.0.1";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "v${version}";
    hash = "sha256-ESsO3cfg3DcscmhlTMYK5h47QR10dmzIsBE8yb1FL5I=";
  };

  cargoHash = "sha256-yR3rItzINUKTZ//nttbxkQC24gmZVl2/nGkyPVs3Nqw=";

  # When installing from source only run unit tests
  checkPhase = ''
    cargo test $UNIT_TEST --lib
  '';

  # NOTE: legacy darwin.apple_sdk.frameworks references were removed in
  # nixpkgs; the default SDK (via SDKROOT) provides CoreServices now.

  meta = with lib; {
    description = "A lightweight watcher";
    homepage = "https://github.com/cristianoliveira/funzzy";
    changelog = "https://github.com/cristianoliveira/funzzy/releases/tag/${src.rev}";
    license = licenses.mit;
    maintainers = with maintainers; [ cristianoliveira ];
  };
}
