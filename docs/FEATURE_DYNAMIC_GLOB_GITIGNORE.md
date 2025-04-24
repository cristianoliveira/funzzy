# FEATURE: Dynamic glob pattern from .gitignore

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
    - "{{git}}" # This uses gitignore from the current folder to match ignore files

- name: example with different path
  run: "make test"
  change: 
    - '**/*'
  ignore:
    - "{{git:another/folder/.gitignore}}" # This uses gitignore from another folder to match ignore files
```
