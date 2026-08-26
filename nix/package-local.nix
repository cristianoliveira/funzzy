{ lib , rustPlatform , fetchFromGitHub }:

rustPlatform.buildRustPackage {
  pname = "funzzy";
  version = "9961199";

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

  cargoHash = "sha256-OfWacQkkdIgVfhA/JmWb2R0j2eA2PZxaMBX3IcfnLrU=";

  # NOTE: legacy darwin.apple_sdk.frameworks references were removed in
  # nixpkgs; the default SDK (via SDKROOT) provides CoreServices now.

# Custom build phase
# NOTE: to debug pass --verbose to cargo test
# and to run a specific test pass --test <test_name>
# cargo test --test ${INTEGRATION_TEST:-'*'} -- --nocapture
# see .watch.yaml
# Creating here the temporary directory in order it to be created with
# the right permissions
  # Deterministic gate: unit tests only. The full integration suite is
  # timing-sensitive (real watchers, subprocess scheduling) and runs on real
  # runners in CI (on-push-integration-test.yml + the agent-final watcher
  # gate). Inside the nix build sandbox task commands run an order of
  # magnitude slower, which makes bounded-timeout integration assertions
  # unreliable; keep the sandbox gate deterministic like the stable package.
  checkPhase = ''
    cargo test $UNIT_TEST --lib
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
    changelog = "https://github.com/cristianoliveira/funzzy/releases";
    license = licenses.mit;
    maintainers = with maintainers; [ cristianoliveira ];
  };
}
