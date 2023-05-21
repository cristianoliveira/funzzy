set -e

ls -la tests/integration/
echo "$HOME"
echo "$PWD"
ls -la "$PWD/tests/integration/"

export TEST_DIR="$PWD/tests/integration"
echo "$TEST_DIR"
export HELPERS="$TEST_DIR/functions.sh"

echo "AAAAAAAAAAAAAAAAAAAAAAAA"
cat "$HELPERS"

cargo build --release

cp target/release/funzzy $TEST_DIR/funzzy

PATH=$PATH:tests/integration

for spec in $TEST_DIR/specs/*; do
  echo "Running $spec"
  sh "$spec" && echo "result: passed" || exit 1
  echo "----------------------------"
done

echo "All integration tests passed"
exit 0
