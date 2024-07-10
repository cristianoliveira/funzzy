{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "1.4.0";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "v${version}";
    hash = "sha256-7rCqz7os9N7R7s3+hAqAafJFa/rLsKdddx4crp93Hzo=";
  };

  cargoHash = "sha256-o/Mr3AEYBDzRz4hWjR/Dy9X4PiQ7kc1YaexYnr2AuW4=";

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
