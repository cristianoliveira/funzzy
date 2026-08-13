{ lib , rustPlatform , fetchFromGitHub }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "1.5.0";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "v${version}";
    hash = "sha256-3EHZvgHlM3ldX6SEyqGf6MZIrDFOLXbKTZnJNczT570=";
  };

  cargoHash = "sha256-n9UHyr7W4hrN0+2dsYAYqkP/uzBv74p5XHU0g2MReJY=";

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
