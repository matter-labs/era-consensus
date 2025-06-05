# Style Guide

## Toolchain

### Rust

We always use the `stable` channel.

### Tests

We prefer to use `cargo nextest` over the default `cargo test`. It is enabled in our CI and it is highly recommended for use in your local machine.

You can install it with the following command:

```
cargo install cargo-nextest --locked
```

And then run tests with:

```
cargo nextest run
```

### Linting and formatting

We use `clippy` to lint the code. You can run it with the following command:

```
cargo clippy --fix --all-targets
```

We use `rustfmt` to format the code, but with custom flags. You can run it with the following command:

```
cargo fmt --all -- --config imports_granularity=Crate --config group_imports=StdExternalCrate --config format_strings=true
```

Both of these are enabled in our CI.

## Panics

We use `[panic=abort]` in this repository (see `Cargo.toml`), so that as soon as `panic` is called, the process aborts.
The backtrace is still printed out to `stderr` before abort. This is only relevant because our application
is multi-threaded as the default behavior (`[panic=unwind]`) would only abort a single thread instead of the whole process.
Aborting the whole process is helpful because:
* `panic` is supposed to mean a bug in the code. If we have a bug and panic occurs, it means that our process is in an
  undefined state. In particular it might misbehave, corrupt the persistent state, do other random stuff. It is safer to just
  stop the execution.
* Uncaught `panic` on the main thread of the process triggers destructors (i.e. `drop()` calls) of the global variables.
  Other threads are stopped only after the main thread is stopped, so there is a brief period of time during which those other
  threads have access to destroyed global variables which is an undefined behavior. For example, there is a warning in RocksDB
  implementation that such an event might lead to database corruption. Abort on panic prevents destroying the global variables
  altogether which is totally fine.
* Our application should be crash-safe (for example, power outage/process preemption might happen at any time) and being exposed
  to crashes due to `panic`s makes it easier to keep a crash-safety mentality in the team, so that no one tries to catch the panics
  in production code.
* If a panic causes a subthread to terminate, it might degrade the performance of the process which might not be immediately observable.
  Imagine the debugging hell such a situation might turn into: a thread has panicked few hours ago, it is hardly impossible to find
  the logs of that event, and a totally unrelated part of the logic is throwing errors which are just a remote consequence of the
  original issue. `[panic=abort]` stops the thread as soon as the problem occurs, which makes debugging way easier.


### panics in tests

Currently `[panic=abort]` is ignored in tests, since otherwise a panic when running `cargo test` would crash the whole test harness.
Moreover it is possible (although we discourage this, see the next section) to expect a panic in some tests. Still, in multithreaded tests it is also
(almost always) useful to fail the test at first panic that occurs. Until `[panic=abort]` is available for `cargo test`, the recommended alternative is to
* use `cargo nextest` which executes each test in a separate subprocess (this way a panic won't kill the test harness itself)
* enable panic on abort in runtime by substituting the panic hook, if `cargo nextest` is detected to be the harness (via env variables).

### `should_panic` is discouraged

APIs shouldn't panic, but rather [return Result](https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html)
(this doesn't apply to panics in test code, but in test code it simply means immediate
test failure, which is consistent with the `panic=abort` semantics).
Panic should be reserved for internal bugs, for example when you call `f(x).unwrap()`,
when you know that for every `x` than can occur at this callsite, `f(x)` will always succeed.
Clean code should never expect a panic. Code which is expecting a panic is usually some meta-code (like test harness itself),
or code which actually is written with `panic=unwind` in mind (and people rarely think about it when coding).

## Structured concurrency

First, read the [README](node/libs/concurrency/README.md) of the `zksync_concurrency` crate to understand the core concepts and basic usage.

### Naming conventions

Scope variable should be called `s` or `scope`.

Variables/arguments of type `&ctx::Ctx` should be called `ctx`. It is important to use the same name
everywhere to avoid accidental use of the wrong context: each code location should have access to at most 1 `&ctx::Ctx`
and variable shadowing helps us with keeping that invariant.

Functions should have a name prefixed with `run_` if they implement a routine
which runs indefinitely unless explicitly cancelled (like an http server, health check, actor).
For example
```rust
async fn run_server(ctx: &ctx::Ctx, ...) -> anyhow::Result<()> { ... }
```

Functions with names prefixed with `spawn_` can spawn tasks and return before they complete.
However since our codebase is using structured concurrency, you shouldn't implement any `spawn_` functions:
for spawning concurrent tasks, use `Scope::spawn/spawn_bg/spawn_blocking/spawn_bg_blocking` functions
which enforce the invariant that the tasks terminate before the scope terminates.

### Avoid passing `Scope` as a function argument.

The point of structured concurrency is to make the concurrency an implementation detail of otherwise synchronous function.
If logic within a `scope::run!` grows too complex, you usually should be able to extract that logic into auxiliary functions
which internally will run another `scope::run!`, and spawn those functions from the main `scope::run!`.
Only if that also fails, you should fallback to pass the scope to the auxiliary functions (and they should be private).

