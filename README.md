# funzzy
The configurable watcher.
Run configured commands for differents directories using semantic yaml.

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

```

## Installing
Make sure you have installed the follow dependecies:
- Rust
```bash
make build
```

## Running
```bash
funzzy watch
```

# Licence
This project was made under MIT Licence.
