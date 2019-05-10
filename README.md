# funzzy  [![Build Status](https://travis-ci.org/cristianoliveira/funzzy.svg?branch=master)](https://travis-ci.org/cristianoliveira/funzzy) [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy)

Yet another fancy watcher. (Inspired by [antr](https://github.com/juanibiapina/antr) / [entr](http://entrproject.org/))

Configure execution of different commands using semantic yaml.

```yaml
# .watch.yaml
# list here all the events and the commands that it should execute
# TIP: include '.watch.yaml' in your .git/info/exclude to ignore it.

- name: run my tests
  run: make test
  change: 'tests/**'
  ignore: 'tests/integration/**'

- name: fast compile sass
  run: compass compile src/static/some.scss
  change: ['src/static/**', 'src/assets/*']

- name: Starwars
  run: telnet towel.blinkenlights.nl
  change: '.watch.yaml'

- name: say hello
  run: say hello
  change: '.watch.yaml'
  run_on_init: true
```

## Motivation
Create a lightweight watcher to run my tests everytime something in my project change.
So I won't forget to keep my tests passing. Funzzy was made with Rust that is why it consumes almost nothing to run.


## Installing

  - OSX:
  ```bash
  brew tap cristianoliveira/tap
  brew update
  brew install funzzy
  ```

  - Linux:
  ```bash
  curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/master/linux-install.sh | sh
  ```

  - With Cargo
  ```bash
  cargo install funzzy
  ```
  *Make sure you have `$HOME/.cargo/bin` in your PATH
  `export $PATH:$HOME/.cargo/bin`

#### From source
Make sure you have installed the follow dependecies:
- Rust

Clone this repo and do:
```bash
make install
```

## Running
Initializing with boilerplate:
```bash
funzzy init
```
Change the yaml as you want. Then run:
```bash
funzzy watch
```

Run with some arbitrary command and stdin
```bash
find . -R '**.rs' | funzzy 'cargo build'
```

Run some arbitrary command in an interval of seconds
```bash
funzzy run 'cargo build' 10
```
## Playground
**It does not work between vm and host machine**

If you wanna try without installing it in your machine, try the playground vagrant.
```bash
cd funzzy
vagrant up

# Testing
vagrant ssh -c "cd /vagrant && funzzy watch"

# Another shell
vagrant ssh -c "touch /vagrant/.watch.yaml"
```
It will take some time to be prepared.

## Tests
Running tests:
```bash
cargo test
```
or simple `make tests`

## Code Style
We use [clippy](https://github.com/Manishearth/rust-clippy) for lintting the funzzy's source code. Make sure you had validate it before commit.

## Contributing
 - Fork it!
 - Create your feature branch: `git checkout -b my-new-feature`
 - Commit your changes: `git commit -am 'Add some feature'`
 - Push to the branch: `git push origin my-new-feature`
 - Submit a pull request

Pull Requests are really welcome! Others support also.

**Pull Request should have unit tests**

# License
This project was made under MIT License.
