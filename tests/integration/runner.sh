set -e

ls -la tests/integration/
echo "$HOME"
echo "$PWD"

export TEST_DIR="tests/integration"
export HELPERS="./functions.sh"

cargo build --release

cp target/release/funzzy tests/integration/funzzy
cd tests/integration

PATH=$PATH:tests/integration

for spec in specs/*; do
  echo "Running $spec"
  sh "$spec" && echo "result: passed" || exit 1
  echo "----------------------------"
done

echo "All integration tests passed"
exit 0
