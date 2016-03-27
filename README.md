# funzzy
The configurable watcher. (Inspired by [antr](https://github.com/juanibiapina/antr) / [entr](http://entrproject.org/))

Run configured commands for different directories using semantic yaml.

Example:
```yaml
# watch.yaml
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
    change: "watch.yaml"
    run: telnet towel.blinkenlights.nl
```

## Motivation
Have you ever used another watcher? 

Well, I did. The last one was Grunt and it consumes almost all of my computer's resources.
Funzzy was made with Rust that is why it consumes almost nothing to run.


## Installing
Make sure you have installed the follow dependecies:
- Rust
```bash
make install
```

## Running
Initializing whit boilerplate:
```bash
funzzy init
```
Then run:
```bash
funzzy watch
```

# Licence
This project was made under MIT Licence.
