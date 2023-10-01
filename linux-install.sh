CARGO_VERSION="$(curl https://raw.githubusercontent.com/cristianoliveira/funzzy/master/Cargo.toml | grep version | awk -F\" '{print $2}')"
VERSION="v${1:-$CARGO_VERSION}"

echo "Installing funzzy $VERSION"

cd /tmp

wget https://github.com/cristianoliveira/funzzy/releases/download/$VERSION/funzzy-$VERSION-x86_64-unknown-linux-gnu.tar.gz
tar -xf funzzy-$VERSION-x86_64-unknown-linux-gnu.tar.gz

sudo cp pkg/funzzy /usr/local/bin
sudo cp pkg/fzz /usr/local/bin

echo "Cli was installed in /usr/local/bin/funzzy and /usr/local/bin/fzz"
echo "To uninstall just run 'rm /usr/local/bin/funzzy' and  'rm /usr/local/bin/fzz'"
