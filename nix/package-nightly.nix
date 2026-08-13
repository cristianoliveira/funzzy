{ lib , rustPlatform , fetchFromGitHub }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "88b89cb";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "${version}";
    hash = "sha256-ZbpZaAoUsHPPbBAOOLYvEJDy06WO0uDYnRF1+gEzB0Q=";
  };

  cargoHash = "sha256-m7qlL+ajw/rwIHQ7KAw7gI9QmpTBnxWEeTVRgrBOcl4=";

  # NOTE: legacy darwin.apple_sdk.frameworks references were removed in
  # nixpkgs; the default SDK (via SDKROOT) provides CoreServices now.

  meta = with lib; {
    description = "A lightweight watcher";
    homepage = "https://github.com/cristianoliveira/funzzy";
    changelog = "https://github.com/cristianoliveira/funzzy/releases";
    license = licenses.mit;
    maintainers = with maintainers; [ cristianoliveira ];
  };
}

