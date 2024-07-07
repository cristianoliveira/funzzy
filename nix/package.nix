{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "master";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    # rev = "master";
    rev = "nix";
    hash = "sha256-+3fMiwF44Uq41ofXxb57vxzXt7H3R3lj7og8IDVxi7w=";
  };

  cargoHash = "sha256-cX/WEsNiqSXH8U13LMo8d4aFV2i+X8SPr6DKwx+tiKI=";

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
