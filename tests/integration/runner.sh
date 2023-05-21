
set -e

cargo build --release

cp target/release/funzzy tests/integration/funzzy
cd tests/integration

export HELPERS="./tests/integration/functions.sh"

PATH=$PATH:./tests/integration

for spec in specs/*; do
  echo "Running $spec"
  sh "$spec" && echo "result: passed" || exit 1
  echo "----------------------------"
done

echo "All integration tests passed"
exit 0
