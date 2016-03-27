# funzzy  [![Build Status](https://travis-ci.org/cristianoliveira/funzzy.svg?branch=master)](https://travis-ci.org/cristianoliveira/funzzy)  [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy)

The configurable watcher. (Inspired by [antr](https://github.com/juanibiapina/antr) / [entr](http://entrproject.org/))

Configure execution of commands when some file change in different directories using semantic yaml.

Example:
```yaml
# .watch.yaml
# list here all the events and the commands that it should execute

- name: run my tests
  when:
    change: 'tests/**'
    run: make test

- name: compile my sass
  when:
    change: 'src/static/**'
    run: compass

- name: Starwars
  when:
    change: ".watch.yaml"
    run: telnet towel.blinkenlights.nl
```

## Motivation
Create a light watcher to run my tests everytime something in my project change. So I won't forget to keep my tests passing. Funzzy was made with Rust that is why it consumes almost nothing to run.


## Installing
Make sure you have installed the follow dependecies:
- Rust
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

## Tests
Running tests:
```bash
cargo test
```

## Contributing
 - Fork it!
 - Create your feature branch: `git checkout -b my-new-feature`
 - Commit your changes: `git commit -am 'Add some feature'`
 - Push to the branch: `git push origin my-new-feature`
 - Submit a pull request

Pull Requests are really welcome! Others support also.

**Pull Request should have unit tests**

# Licence
This project was made under MIT Licence.
