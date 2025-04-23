## FEATURE: External file dynamic glob

---
minimal-version: nightly
---

This feature allows one to define a dynamic glob pattern in the `change` and `ignore` field of a task. Allows you to use external files, like `.gitignore` as pattern for watch rules

### USAGE

In the example below, the `change` field for the first task uses a static glob pattern `dist/**/*.js` and a dynamic glob pattern loaded from `.gitignore`. It will load the content
of `.gitignore` and use it as a list of glob patterns to watch for changes.

In order to use the feature declare one or more pattern with the patterrn `{{file:filenamepath}}` in the `change` or `ignore` field. The filename can be relative to the current working directory or absolute.

```yaml
- name: compile code
  run: "make all"
  change: 
    - dist/**/*.js # This is a static glob pattern
    - "{{file:.gitignore}}" # This loads the .gitignore file and uses it as a glob patterns list

- name: run tests
  run: "npm test"
  change: "tests/**/*.js"
```

Regarding the file format, the dynamic glob pattern should be a list of glob patterns, one per line. For example:

```
# .gitignore
dist/**/*.js
dist/**/*.css 
```

### EXAMPLE

Given the following `.watch.yaml` file:

```yaml
- name: compile code
  run: "make all"
  change: 
    - '**/*.js' # This is a static glob pattern
  ignore:
    - "{{file:.gitignore}}" # This loads the .gitignore file and uses it as a glob patterns list
```

And the following `.gitignore` file:
```text
node_modules/
__coverage__/
```

Once you run `fzz`, it will watch for changes in the `dist/**/*.js` files and ignore any changes in the `node_modules/` and `__coverage__/` directories.

### FEATURE: Dynamic glob pattern from .gitignore

This feature allows one to define a dynamic glob pattern in the `ignore` field of a task. Allows you to use external files, like `.gitignore` as pattern for watch rules

**Important**  
It will use git glob like pattern instead of unix glob pattern. 
For example, `**/node_modules` will match any directory named `node_modules` in the project, while `node_modules/**` will match any file or directory inside the `node_modules` directory.

### EXAMPLE

Given the following `.watch.yaml` file:

```yaml
- name: compile code
  run: "make all"
  change: 
    - '**/*.js' # This is a static glob pattern
  ignore:
    - "{{gitignore:./.gitignore}}" # This uses gitignore to match ignore files
```
