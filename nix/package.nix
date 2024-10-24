{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "1.4.1";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "v${version}";
    hash = "sha256-R1NJM/jZxeFIXfzbmQISw8VhR0KtZGTntqiLpA9Pup8=";
  };

  cargoHash = "sha256-SigVnhrd52eS1oC6jpovCCpItRQbr8lw4WaSPsuB4Ks=";

  # When installing from source only run unit tests
  checkPhase = ''
    cargo test $UNIT_TEST --lib
  '';

  buildInputs = lib.optionals stdenv.isDarwin [
    darwin.apple_sdk.frameworks.CoreServices
  ];

  meta = with lib; {
    description = "A lightweight watcher";
    homepage = "https://github.com/cristianoliveira/funzzy";
    changelog = "https://github.com/cristianoliveira/funzzy/releases/tag/${src.rev}";
    license = licenses.mit;
    maintainers = with maintainers; [ cristianoliveira ];
  };
}
