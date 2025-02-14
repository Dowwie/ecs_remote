This is a utility to help you remote into an ECS Fargate Task configured for ECSExec.  Without this utility, you need to jump through a few hoops to connect.



The AWS Session Manager Plugin is required for the execute-command feature to work, as it handles the interactive session.
Without this plugin installed, the `aws ecs execute-command` will fail to establish an interactive session. Would you like me to help you with the installation for your specific operating system?

To install in mac:

- `brew install session-manager-plugin`
- install rust: https://www.rust-lang.org/tools/install
- from the root directory of the project, compile for your targeted system: `cargo build --release` , and the executable will be at /target/release/ecs_remote
- `chmod +x ecs_remote`

You need to have valid aws sso credentials set up. In the example below, I call my sso profile 'uat-admin' for test-admin

`ecs_remote -h`

```bash
ECS Execute Command utility for connecting to running tasks

Usage: ecs_remote [OPTIONS]

Options:
  -p, --profile <PROFILE>      AWS Profile name to use [default: default]
  -l, --cluster <CLUSTER>      Target cluster name or ARN
  -t, --container <CONTAINER>  Container name to execute command in
  -h, --help                   Print help
  -V, --version                Print version

Example usage:
    export AWS_PROFILE=uat-admin

    ecs_remote -t {container} -p uat-admin
```
