# funzzy (fzz) [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy) [![CI integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml) [![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)

A lightweight watcher inspired by [antr](https://github.com/juanibiapina/antr) and [entr](https://github.com/eradman/entr). See also: [funzzy.nvim](https://github.com/cristianoliveira/funzzy.nvim)

Configure auto-execution of different commands using semantic YAML and [Unix shell style pattern match](https://en.wikipedia.org/wiki/Glob_(programming)) or stdin.

For a workflow as simple as:
```bash
find . -name '*.ts' | funzzy 'npx eslint .'
```

Or more complex workflows like:
```yaml
# .watch.yaml (or .watch.yml)
# list here all the events and the commands that it should execute
# TIP: include '.watch.yaml' in your .git/info/exclude to ignore it.
# TIP2: List the tasks/steps from quicker to slower for better workflows
#
# Run: `fzz` to start this workflow

- name: run my tests
  run: make test
  change: "tests/**"
  ignore: "tests/integration/**"
  run_on_init: true

- name: Starwars ascii art
  run: telnet towel.blinkenlights.nl
  change: ".watch.yaml"

# Command templates for custom scripts
- name: run test & linter for a single file
  run: 
   - "npm run lint -- {{filepath}}",
   - "npm test -- $(echo '{{filepath}}' | sed -r s/.(j|t)sx?//)"
  change: ["src/**", "libs/**"]
  ignore: ["src/**/*.stories.*", "libs/**/*.log"]

- name: finally stage the changed files in git
  run: 'git add $(echo "{{filepath}}" | sed "s#$PWD/##")'
  change: 
    - "src/**"
    - "tests/**"
  ignore: "**/*.log"
```

Want more examples? 

 - [Check our workflow in funzzy](https://github.com/cristianoliveira/funzzy/blob/master/.watch.yaml#L6) :)
 - [Check the examples folder](https://github.com/cristianoliveira/funzzy/tree/master/examples)

## Motivation

To create a lightweight watcher that **allows me to set up personal local workflows with specific automated checks and steps, similar to GitHub Actions**. 
Funzzy was built with Rust, which makes it blazingly fast and light.

## Installing

### OSX:

```bash
brew tap cristianoliveira/tap
brew update
brew install funzzy
```

### Linux:

```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/master/linux-install.sh | sh
```

You can specify the versions:
```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/master/linux-install.sh | bash - 1.0.0
```

### Nix
  
```bash
nix-env -iA nixpkgs.funzzy
```

or, if you use `shell.nix`:
  
  ```nix
{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    buildInputs = [
      pkgs.funzzy
    ];
  };
```

### With Cargo

```bash
cargo install funzzy
```

\*Make sure you have `$HOME/.cargo/bin` in your PATH
`export PATH=$HOME/.cargo/bin:$PATH`

- From source

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

Fail fast which bails the execution if any task fails. Useful for workflows that
depend on all task to be successful. [See its usage in our workflow](https://github.com/cristianoliveira/funzzy/blob/master/.watch.yaml#L6)

```bash
fzz --fail-fast
```

Filtering tasks by target. 

```bash
fzz -t "@quick"
# Assuming you have one or more tasks with `@quick` in the name, it will only load those tasks
```

Run with some arbitrary command and stdin

```bash
find . -name '*.rs' | fzz 'cargo build'
```

Templates for composing commands

```bash
find . -name '*.[jt]s' | fzz 'npx eslint {{filepath}}'
```

Run in "non-block" mode, which cancels the currently running task when there are new change events from files.
It's super useful when a workflow contains long-running tasks. [See more in long task test](https://github.com/cristianoliveira/funzzy/blob/2e6b53b8af3c3d85f193ec6abb49bd8450f31c83/tests/watching_with_non_block_flag.rs#L7)

```bash
fzz --non-block
```

## Troubleshooting

#### Why the watcher is running the same task multiple times?

This might be due to different causes, the most common issue when using VIM is because of its default backup setting
which causes changes to multiple files on save. (See [Why does Vim save files with a ~ extension?](https://stackoverflow.com/questions/607435/why-does-vim-save-files-with-a-extension/607474#607474)).
For such cases either disable the backup or [ignore them in your watch rules](https://github.com/cristianoliveira/funzzy/blob/master/examples/long-task.yaml#L5).

For other cases use the verbose `fzz -V | grep 'Triggered by'` to understand what is triggering a task to be executed.

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

We use `rustfmt` to format the code. To format the code run:

```bash
cargo fmt
```

## Contributing

- Fork it!
- Create your feature branch: `git checkout -b my-new-feature`
- Commit your changes: `git commit -am 'Add some feature'`
- Push to the branch: `git push origin my-new-feature`
- Submit a pull request

### Want to help?

 - Open pull requests
 - Create Issues
 - Report bugs
 - Suggest new features or enhancements

Any help is appreciated!

**Pull Request should have unit tests**

# License

This project was made under MIT License.
