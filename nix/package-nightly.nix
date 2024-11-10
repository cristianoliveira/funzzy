{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "640767f";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "${version}";
    hash = "sha256-w9inuHXrnANoAbWXVxKgCriVfKYJTTsRFGyow382TvQ=";
  };

  cargoHash = "sha256-vOnmQYnltUh4kfgUgMwkPPA1vNOwTpmFLG9r9R/j7JM=";

  buildInputs = lib.optionals stdenv.isDarwin [
    darwin.apple_sdk.frameworks.CoreServices
  ];

  meta = with lib; {
    description = "A lightweight watcher";
    homepage = "https://github.com/cristianoliveira/funzzy";
    changelog = "https://github.com/cristianoliveira/funzzy/releases";
    license = licenses.mit;
    maintainers = with maintainers; [ cristianoliveira ];
  };
}

