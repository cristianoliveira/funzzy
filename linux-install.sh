VERSION="v0.3.1"

cd /tmp

wget https://github.com/cristianoliveira/funzzy/releases/download/$VERSION/funzzy-v0.3.0-x86_64-unknown-linux-gnu.tar.gz
tar -xf funzzy-$VERSION-x86_64-unknown-linux-gnu.tar.gz
sudo cp funzzy /usr/local/bin

echo "Application was installed in /usr/local/bin. To uninstall just do rm /usr/local/bin/funzzy"
