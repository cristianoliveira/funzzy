# FEATURE: Dynamic glob pattern from gitignore

This feature allows one to define a dynamic glob pattern in the `ignore: {{git}}` for a task and it will load the content of `.gitignore` file and use it as a list of glob patterns to ignore. The pattern applied will be a git like ignore.

 - By default it will ignore the `.git/` directory.
 - You can specify a different path to the `.gitignore` file by using `{{git:path/to/.gitignore}}`.

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
    - "{{git}}" # This uses gitignore from the current folder to match ignore files

- name: example with different path
  run: "make test"
  change: 
    - '**/*'
  ignore:
    - "{{git:another/folder/.gitignore}}" # This uses gitignore from another folder to match ignore files
```

And the following `.gitignore` file:

```text
node_modules/
__coverage__/
```

When running the watcher it will trigger for any changes in the `dist/**/*.js` files and ignore any changes in the `node_modules/` and `__coverage__/` directories.
