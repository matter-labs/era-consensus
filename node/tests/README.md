# Tests overview

This module contains was designed as a binary crate with the tests to be run against the nodes running in kubernetes. This tests also run inside the same k8s cluster as the nodes in a separate Pod. 

## How to run the tests Pod

1. First you have to generate the executable for the `tester` to use it inside the kubernetes Pod. To do this use the following command:
    > This could take a while but it'll be necessary only for the first time, if you already ran it, then skip this step and go ahead with the next one.

    ```
    make docker_build_tester
    ```
2. Use the following command to run the tests. This command will make a deployment, creating a pod in the kubernetes cluster and run the tests inside it
   ```
   make start_k8s_tests
   ```

