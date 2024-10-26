{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "47eb77e";

  src = fetchFromGitHub {
    owner = "cristianoliveira";
    repo = "funzzy";
    rev = "${version}";
    hash = "sha256-3EHZvgHlM3ldX6SEyqGf6MZIrDFOLXbKTZnJNczT570=";
  };

  cargoHash = "sha256-7Yql5D+g+xOSX3hQTlV1gGBBSfXFIRSkt3MyRp/bgII=";

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

