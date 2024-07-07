{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "master";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "master";
    hash = "sha256-YjA/XxVB8gGxyLovxTTatSC/ESBCkgz7Not0qfEPxtw=";
  };

  cargoHash = "sha256-CN+ZM0AVy4V5vVoI7AS49oEhVXTuJ412fDGC0qjEUKQ=";

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
