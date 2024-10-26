{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "1.5.0";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "v${version}";
    hash = "sha256-3EHZvgHlM3ldX6SEyqGf6MZIrDFOLXbKTZnJNczT570=";
  };

  cargoHash = "sha256-DJGpNuRb0bFdXBV/p3enyTvZgJIQOaNy3kijxuLFl1g=";

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
