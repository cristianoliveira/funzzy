{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "master";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "master";
    hash = "sha256-FYGiG9s4TLUgheoY9js2uIhxNa7FvVQnkieM4JfeO7E=";
  };

  cargoHash = "sha256-tVgEPB7SPW5QU0cHgok4k93DZagZyQhf44jeUDWxSFg=";

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
