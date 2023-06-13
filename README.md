# funzzy [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy) [![CI integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml) [![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)

Yet another fancy watcher. (Inspired by [antr](https://github.com/juanibiapina/antr) / [entr](http://entrproject.org/))

Configure execution of different commands using semantic YAML and [Unix shell style pattern match](https://en.wikipedia.org/wiki/Glob_(programming)). See also [funzzy.nvim](https://github.com/cristianoliveira/funzzy.nvim)

```yaml
# .watch.yaml
# list here all the events and the commands that it should execute
# TIP: include '.watch.yaml' in your .git/info/exclude to ignore it.

- name: run my tests
  run: make test
  change: "tests/**"
  ignore: "tests/integration/**"

# Command templates for custom scripts
- name: run test & linter for single file
  run: [
    "npm run lint -- {{filepath}}",
    "npm test -- $(echo '{{filepath}}' | sed -r 's/\.(j|t)sx?//')"
  ]
  change: ["src/**", "libs/**"]
  ignore: ["src/**/*.stories.*", "libs/**.log"]

- name: Starwars
  run: telnet towel.blinkenlights.nl
  change: ".watch.yaml"

- name: say hello
  run: echo "hello on init"
  change: "./*.yaml"
  run_on_init: true
```

## Motivation

Create a lightweight watcher to run my tests every time something in my project change.
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

\*Make sure you have `$HOME/.cargo/bin` in your PATH
`export $PATH:$HOME/.cargo/bin`

#### From source

Make sure you have installed the follow dependencies:

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

Change the YAML as you want. Then run:

```bash
funzzy
```

Filtering task by target:

```bash
funzzy --target="my task"
```

Run with some arbitrary command and stdin

```bash
find . -R '**.rs' | funzzy 'cargo build'
```

Templates for composing commands

```bash
find . -R '**.rs' | funzzy 'cargo lint {{filepath}}'
```

See more on [examples](https://github.com/cristianoliveira/funzzy/tree/master/examples)
or in [the integration specs](https://github.com/cristianoliveira/funzzy/tree/master/tests/integration/specs)

## Troubleshooting

#### Why the watcher is running the same task multiple times?

This might be due to different causes, the most common issue when using VIM is because of the default backup setting
which causes changes to multiple files on save. See [Why does Vim save files with a ~ extension?](https://stackoverflow.com/questions/607435/why-does-vim-save-files-with-a-extension/607474#607474).
For such cases either disable the backup or [ignore them in your watch rules](https://github.com/cristianoliveira/funzzy/blob/master/examples/long-task.yaml#L5).

For other cases use verbose `funzzy -V` to undersand what is triggering a task to be executed.

## Automated tests

Running unit tests:

```bash
cargo test
```

or simple `make tests`

Running integration tests:

```
make integration
```

## Code Style

We use [clippy](https://github.com/Manishearth/rust-clippy) for lintting the funzzy's source code. Make sure you had validated it before commit.

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
