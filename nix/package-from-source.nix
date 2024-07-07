{ lib , rustPlatform , fetchFromGitHub , stdenv , darwin }:

rustPlatform.buildRustPackage rec {
  pname = "funzzy";
  version = "nightly-20240707a";

  ## build with local source
  src = ../.;

  # Only rebuild when theres a diff in the Cargo.lock
  # cargoPatches = [
  #   ./gitdiff.patch  # Path to your patch file
  # ];

# NOTE: To limit the build for changes in the Cargo.lock
# cargoDeps = rustPlatform.importCargoLock {
#   lockFile = (lib.builtins.toFile "Cargo.lock");
#   allowBuiltinFetchGit = true;
# };

  cargoHash = "sha256-CN+ZM0AVy4V5vVoI7AS49oEhVXTuJ412fDGC0qjEUKQ=";

  buildInputs = lib.optionals stdenv.isDarwin [
    darwin.apple_sdk.frameworks.CoreServices
  ];

# Custom build phase
# NOTE: to debug pass --verbose to cargo test
# and to run a specific test pass --test <test_name>
# cargo test --test ${INTEGRATION_TEST:-'*'} -- --nocapture
# see .watch.yaml
# Creating here the temporary directory in order it to be created with
# the right permissions
  checkPhase = ''
    mkdir -p /tmp/fzz

    ls -la /tmp/fzz

    touch /tmp/fzz/accepts_full_or_relativepaths.txt
    touch /tmp/fzz/accepts_full_or_relativepaths2.txt
    touch /tmp/fzz/accepts_full_or_relativepaths3.txt

    RUST_BACKTRACE=1 cargo test --test '*' -- --nocapture

    rm -rf /tmp/fzz
  '';

  # Common commands here
  #
  #   RUST_BACKTRACE=1 cargo test --test watching_arbitrary_files_running_arbitrary_commands -- --nocapture
  #   cargo test --test '*' -- --nocapture
  #

  # NOTE: as last resource, you can disable the tests
  # May need to disable tests because it requires
  # creating files and directories
  # doCheck = false;

  meta = with lib; {
    description = "A lightweight watcher";
    homepage = "https://github.com/cristianoliveira/funzzy";
    changelog = "https://github.com/cristianoliveira/funzzy/releases/tag/${src.rev}";
    license = licenses.mit;
    maintainers = with maintainers; [ cristianoliveira ];
  };
}
