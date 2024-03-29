# funzzy (fzz) [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy) [![CI integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml) [![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)

Yet another fancy watcher. (Inspired by [antr](https://github.com/juanibiapina/antr) / [entr](https://github.com/eradman/entr)). See also [funzzy.nvim](https://github.com/cristianoliveira/funzzy.nvim)

Configure auto-execution of different commands using semantic YAML and [Unix shell style pattern match](https://en.wikipedia.org/wiki/Glob_(programming)) or stdin.

As simple as
```bash
find . -name '*.ts' | funzzy 'npx eslint .'
```

Or complicated as
```yaml
# .watch.yaml
# list here all the events and the commands that it should execute
# TIP: include '.watch.yaml' in your .git/info/exclude to ignore it.

- name: run my tests
  run: make test
  change: "tests/**"
  ignore: "tests/integration/**"

- name: Starwars
  run: telnet towel.blinkenlights.nl
  change: ".watch.yaml"

- name: say hello
  run: echo "hello on init"
  change: "./*.yaml"
  run_on_init: true

# Command templates for custom scripts
- name: run test & linter for a single file
  run: [
    "npm run lint -- {{filepath}}",
    "npm test -- $(echo '{{filepath}}' | sed -r s/.(j|t)sx?//)"
  ]
  change: ["src/**", "libs/**"]
  ignore: ["src/**/*.stories.*", "libs/**/*.log"]
```

## Motivation

Create a lightweight watcher to run my tests every time something in my project change.
So I won't forget to keep my tests passing. Funzzy was made with Rust which is why it consumes almost nothing to run.

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

Make sure you have installed the following dependencies:

- Rust
- Cargo

Execute:
```
cargo install --git https://github.com/cristianoliveira/funzzy.git
```

Or, clone this repo and run:

```bash
make install
```

## Running

Initializing with boilerplate:

```bash
funzzy init
```

Change the config file `.watch.yaml` as you want. Then run:

```bash
funzzy
# or use the short version
fzz
```

### Options

Use a different config file:

```bash
fzz -c ~/watch.yaml
```

Filtering task by target (contais in task name):

```bash
fzz -t "my task"
```

Run with some arbitrary command and stdin

```bash
find . -name '*.rs' | fzz 'cargo build'
```

Templates for composing commands

```bash
find . -name '*.[jt]s' | fzz 'npx eslint {{filepath}}'
```

Running in "non-block" mode which cancels the currently running task once something changes
super useful if you need to run a long task and don't want to wait for it to finish after a change in the code.
See: [long task test](https://github.com/cristianoliveira/funzzy/blob/master/tests/integration/specs/long-tasks-test.sh)
```bash
fzz --non-block
```

See more in [examples](https://github.com/cristianoliveira/funzzy/tree/master/examples)
or in [the integration specs](https://github.com/cristianoliveira/funzzy/tree/master/tests/integration/specs)

## Troubleshooting

#### Why the watcher is running the same task multiple times?

This might be due to different causes, the most common issue when using VIM is because of the default backup setting
which causes changes to multiple files on save. See [Why does Vim save files with a ~ extension?](https://stackoverflow.com/questions/607435/why-does-vim-save-files-with-a-extension/607474#607474).
For such cases either disable the backup or [ignore them in your watch rules](https://github.com/cristianoliveira/funzzy/blob/master/examples/long-task.yaml#L5).

For other cases use the verbose `funzzy -V` to understand what is triggering a task to be executed.

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

We use [clippy](https://github.com/Manishearth/rust-clippy) for linting the funzzy's source code. Make sure you had validated it before committing.

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
