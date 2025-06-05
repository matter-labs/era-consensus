# `zksync_concurrency`

The `zksync_concurrency` crate provides a structured concurrency framework inspired by Golang's context and errgroup, tailored for Rust. It aims to simplify the management of concurrent tasks, their cancellation, and error propagation in asynchronous and synchronous code.

## Core Concepts

### 1. `ctx::Ctx` - Context

The `ctx::Ctx` (Context) is a fundamental concept in this crate. It acts as a carrier for cancellation signals, deadlines, and other request-scoped values.

- **Cancellation**: A `Ctx` can be canceled, signaling all operations associated with it to terminate gracefully as soon as possible.
- **Deadlines & Timeouts**: A `Ctx` can have a deadline, after which it is automatically canceled. You can create child contexts with shorter timeouts.
- **Propagation**: Contexts are typically passed down the call stack. Functions that perform potentially long-running operations or I/O should accept a `&ctx::Ctx` argument.
- **Clock Abstraction**: `Ctx` provides access to a `Clock` (`ctx.now()`, `ctx.now_utc()`, `ctx.sleep()`), which can be a `RealClock`, or for testing `ManualClock` and `AffineClock`.
- **Random Number Generator**: `Ctx` also provides access to a `Rng` (`ctx.rng()`), which can be a OS-level RNG for production or a deterministic RNG for testing.

**Usage:**

```rust
use zksync_concurrency::{ctx, time};

async fn some_long_operation(ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
    // Perform some work
    // ...

    // Use ctx.wait to await a future while also listening for cancellation
    let result = ctx.wait(async {
        // some sub-operation
        // ...
    }).await?;
    println!("Sub-operation result: {}", result);

    // Or, periodically check if the context is active
    if !ctx.is_active() {
        println!("Context was canceled, cleaning up.");
        return Err(ctx::Canceled);
    }

    // Sleep using the context's clock
    ctx.sleep(time::Duration::milliseconds(100)).await?;

    Ok(())
}

async fn my_task(parent_ctx: &ctx::Ctx) {
    // Create a child context with a timeout
    let ctx_with_timeout = &parent_ctx.with_timeout(time::Duration::seconds(5));

    match some_long_operation(ctx_with_timeout).await {
        Ok(()) => println!("Operation completed successfully."),
        Err(ctx::Canceled) => println!("Operation was canceled."),
    }
}

#[tokio::main]
async fn main() {
    let root_ctx = ctx::root(); // In tests, we use ctx::test_root() instead.
    my_task(&root_ctx).await;
}
```

### 2. `scope` - Structured Concurrency

Structured concurrency ensures that concurrent tasks are managed within a well-defined scope. If a scope exits, all tasks spawned within it are guaranteed to have completed or been canceled.

- **`scope::run!` (async) and `scope::run_blocking!` (sync)**: These macros create a new scope. The scope manages a group of tasks.
    - `scope::run!(ctx, async_closure)`: Use this for asynchronous scopes. The provided closure must be `async`. The `run!` macro itself returns a future that you `.await`. Inside, you'll typically use `s.spawn()` for async tasks and `s.spawn_blocking()` for tasks that need to run on a separate thread due to blocking operations.
    - `scope::run_blocking!(ctx, sync_closure)`: Use this for synchronous scopes. The provided closure is a regular (non-`async`) closure. The `run_blocking!` macro is a blocking call. Inside, you'll typically use `s.spawn_blocking()` for tasks, though `s.spawn()` for async sub-tasks is also possible.
    The choice depends on whether the function creating the scope is `async` or synchronous.
- **Task Spawning within a Scope**:
    - **Main Tasks**: `s.spawn(async_fn)` or `s.spawn_blocking(sync_fn)`. The scope waits for all main tasks to complete. If any main task returns an error or is canceled, the scope is canceled.
    - **Background Tasks**: `s.spawn_bg(async_fn)` or `s.spawn_bg_blocking(sync_fn)`. The scope's primary work is considered complete once all main tasks finish. At this point (or if any task errors, or if the scope is explicitly canceled), the scope's context is canceled. Background tasks are also subject to this cancellation and must terminate gracefully. The `scope::run!` or `scope::run_blocking!` call will only return after all tasks, including background tasks, have fully terminated. They are useful for auxiliary operations like logging, monitoring, or handling side effects that don't define the main completion criteria of the scope.
- **Cancellation**:
    - **Automatic**: If any task (main or background) within the scope returns an `Err`, the scope's context is immediately canceled. All other tasks within that scope should then terminate gracefully.
    - **Explicit**: You can explicitly cancel a scope's context using `s.cancel()`.
- **Error Handling**:
    - The `scope::run!` or `scope::run_blocking!` macro returns a `Result`.
    - If all tasks succeed, it returns the `Ok` result of the root task provided to the macro.
    - If any task fails (returns an `Err`), the macro returns the error of the *first* task that failed. Subsequent errors from other tasks are ignored.
    - If a task panics, the panic is propagated after all other tasks in the scope have completed (or been canceled and terminated).
- **`JoinHandle`**: When you spawn a task, you get a `JoinHandle`. You can use `handle.join(ctx).await` to get the `Result` of that specific task. If the task itself errored, joining it will result in `Err(ctx::Canceled)` because the scope (and thus the task's specific context for error reporting) would have been canceled.

**Usage:**

```rust
use zksync_concurrency::{ctx, scope, time};
use std::sync::atomic::{AtomicUsize, Ordering};

async fn application_logic(ctx: &ctx::Ctx) -> Result<String, anyhow::Error> {
    // scope::run! takes the parent context and a closure.
    // The closure receives the scope's context and a reference to the scope itself.
    let result_from_scope: Result<String, anyhow::Error> = scope::run!(ctx, |ctx, s| async {
        // Spawn a main async task
        s.spawn(async {
            ctx.sleep(time::Duration::milliseconds(50)).await?;
            println!("Main async task 1 completed.");
            // Tasks should return Result<T, E> where E is the scope's error type
            Ok(())
        });

        // Spawn another main blocking task
        let handle = s.spawn_blocking(|| {
            // Simulate work
            std::thread::sleep(std::time::Duration::from_millis(100));
            println!("Main blocking task completed.");

            // Example of returning an error:
            // return Err(anyhow::anyhow!("Something went wrong in blocking task"));
            Ok(100)
        });

        // Spawn a background task
        s.spawn_bg(async {
            for i in 0..5 {
                if !ctx.is_active() {
                    println!("Background task: scope canceled, exiting.");
                    break;
                }
                println!("Background task alive: {}", i);
                ctx.sleep(time::Duration::milliseconds(30)).await?;
            }
            Ok(())
        });

        // Optionally, join a specific task
        match handle.join(ctx).await {
            Ok(val) => println!("Blocking task successfully returned: {}", val),
            Err(ctx::Canceled) => println!("Blocking task was canceled or scope failed."),
        }

        // The root task of the scope. Its Ok value is returned if all tasks succeed.
        // If we want the scope to fail, this root task can return an error,
        // or any spawned task can return an error.
        Ok("All main tasks launched".to_string())
    }).await;

    println!("Scope finished.");
    result_from_scope
}
```

### Error Types

- **`ctx::Canceled`**: A distinct error type indicating that an operation was specifically canceled because its context was terminated. This is crucial for distinguishing cancellations from other failures.

- **`ctx::Error`**: This is an enum that represents the outcome of a cancellable operation. It can be one of two variants:
    - `Error::Canceled(ctx::Canceled)`: Indicates the operation was canceled via its context.
    - `Error::Internal(anyhow::Error)`: Indicates the operation failed for a reason other than cancellation. The underlying cause is captured as an `anyhow::Error`, allowing for rich error reporting from various sources.
   `ctx::Error` is the standard error type for many functions and methods within the `zksync_concurrency` crate.

- **`ctx::Result<T>`**: A convenient type alias for `std::result::Result<T, ctx::Error>`.

- **`error::Wrap`**: A trait similar to `anyhow::Context` for adding context to your custom error types that might embed `anyhow::Error`, while correctly handling `ctx::Canceled`.

```rust
use zksync_concurrency::{ctx, error::Wrap, time};

async fn process_data_with_proper_context(ctx: &ctx::Ctx, data_input: &str) -> ctx::Result<String> {
    if !ctx.is_active() {
        // This becomes ctx::Error::Canceled.
        // The .wrap() here will preserve it as Canceled.
        return Err(ctx::Canceled.into()).wrap(|| format!("Ctx canceled before processing '{}'", data_input));
    }

    if data_input == "trigger_fail" {
        // This becomes ctx::Error::Internal wrapping an anyhow::Error.
        let internal_failure = anyhow::anyhow!("Specific failure for '{}'", data_input);
        return Err(ctx::Error::Internal(internal_failure))
            .wrap(|| format!("High-level context for processing '{}'", data_input));
    }
      
    // Simulating some cancellable work
    ctx.wait(async { tokio::time::sleep(time::Duration::milliseconds(10)).await }).await?;

    ctx::Ok(format!("Successfully processed '{}'", data_input))
}
```

### Pitfalls: `ctx::Canceled` vs. `anyhow::Error`

It is critical to handle `ctx::Canceled` correctly to maintain its distinct semantic meaning. Improperly mixing `ctx::Error::Canceled` with general `anyhow::Error` propagation or the `anyhow::Context` trait can obscure the specific reason for termination (i.e., cancellation).

1.  **Using `anyhow::Context::context()` on `ctx::Error` (or `ctx::Result`)**: 
    - If you have a `ctx::Result<T>` (which is `Result<T, ctx::Error>`) and you call `.context("some context")` on it, a `ctx::Error::Canceled` variant will be converted into an `anyhow::Error`.
    - The resulting `anyhow::Error` will contain the new context string prepended to the string representation of the original `Canceled` error (e.g., "some context: canceled").
    - **Crucially, the distinct `ctx::Error::Canceled` variant is lost.** The error is now a generic `anyhow::Error`, and you can no longer specifically match on `ctx::Error::Canceled`.
    - **Recommendation**: **Always use `your_ctx_result.wrap("...")`** (from `zksync_concurrency::error::Wrap`) when you want to add context to a `ctx::Result` while preserving the `ctx::Error::Canceled` variant if it occurs. If the function must return `anyhow::Result`, be aware of the conversion (see next point).

    ```rust
    use zksync_concurrency::{ctx, error::Wrap as _};
    use anyhow::Context as _;

    let canceled_result: ctx::Result<()> = Err(ctx::Error::Canceled(ctx::Canceled));

    // INCORRECT - converts Canceled to generic anyhow::Error:
    let wrong = canceled_result.clone().context("Added context");
    // `wrong` is now Result<(), anyhow::Error>, cannot match on ctx::Error::Canceled

    // CORRECT - preserves Canceled type:
    let correct = canceled_result.wrap("Added context");
    match correct {
        Err(ctx::Error::Canceled(_)) => { /* Handle cancellation specifically */ }
        Err(ctx::Error::Internal(_)) => { /* Handle other errors */ }
        Ok(_) => {}
    }
    ```

2.  **Propagating `ctx::Error::Canceled` via `?` into a function returning `Result<_, anyhow::Error>`**:
    - If a function `func_A()` returns `ctx::Result<T>` and this result is `Err(ctx::Error::Canceled)`, and you use `func_A()?` inside another function `func_B()` that is declared to return `Result<U, anyhow::Error>`. The `?` operator will perform an implicit conversion from `ctx::Error` to `anyhow::Error` (because `ctx::Error` implements `Into<anyhow::Error>`). The `ctx::Error::Canceled` will be converted into a generic `anyhow::Error`. The error message of this `anyhow::Error` will typically be just "canceled".
    - **The specific `ctx::Canceled` type information is lost** at the level of `func_B()`'s return type. `func_B()` now only knows it got *an* `anyhow::Error`, not specifically a cancellation.
    - **Recommendation**: The **primary and most robust approach** is to **explicitly handle the `ctx::Result`** returned by `func_A()` *before* it's propagated further using `?`, especially if `func_B()` needs to return a generic `Result<_, anyhow::Error>`. This typically involves using a `match` statement on the result of `func_A()`:
        - If `Err(ctx::Error::Canceled)` is received, `func_B()` can perform cancellation-specific cleanup, log the cancellation, or transform it into an appropriate `anyhow::Error` if cancellation at this specific point in `func_B`'s logic is unexpected and should be reported as a distinct failure (while still acknowledging its origin if possible).
        - If `Err(ctx::Error::Internal)` is received, it can be propagated (e.g., using `?` if `func_B` returns `anyhow::Error`) or wrapped as needed.
        - If `Ok` is received, proceed as normal.
      Allowing the `?` operator to implicitly convert `ctx::Error::Canceled` to a generic `anyhow::Error` (when `func_B` returns `Result<_, anyhow::Error>`) should be a conscious decision. It is only appropriate if `func_B` can genuinely treat a context cancellation originating from `func_A` identically to any other internal error, and the loss of the specific `Canceled` type information is acceptable for all callers of `func_B`. If precise cancellation information *must* be propagated out of `func_B`, then `func_B` itself should consider returning `ctx::Result` or a custom error enum that can distinctly represent cancellation (as shown in Scenario 2 of the conceptual example below).

    ```rust
    use zksync_concurrency::{ctx, time};

    fn operation_that_might_be_canceled(ctx: &ctx::Ctx) -> ctx::Result<String> {
        ctx.sleep(time::Duration::milliseconds(1)).await?; // This can return Err(ctx::Error::Canceled)

        // Or, explicitly:
        if !ctx.is_active() {
            return Err(ctx::Canceled); // Becomes ctx::Error::Canceled
        }

        ctx::Ok("Operation Succeeded".to_string())
    }

    // This function loses the specific Canceled type if operation is canceled.
    fn calling_func_that_returns_anyhow(ctx: &ctx::Ctx) -> Result<String, anyhow::Error> {
        let result = operation_that_might_be_canceled(ctx)?; // Converts ctx::Error::Canceled to anyhow::Error
        Ok(format!("Got: {}", result))
    }

    // This function preserves the Canceled type.
    fn calling_func_that_returns_ctx_result(ctx: &ctx::Ctx) -> ctx::Result<String> {
        let result_string = operation_that_might_be_canceled(ctx)?;
        ctx::Ok(format!("Got: {}", result_string))
    }

    // Example of explicit handling when returning anyhow::Error:
    async fn handler_with_explicit_handling(ctx: &ctx::Ctx) -> Result<String, anyhow::Error> {
        match operation_that_might_be_canceled(ctx) {
            Ok(result) => Ok(format!("Processed: {}", result)),
            Err(ctx::Error::Canceled(_)) => {
                // Handle cancellation specifically
                Err(anyhow::anyhow!("Operation was canceled"))
            }
            Err(ctx::Error::Internal(err)) => {
                // Propagate other errors
                Err(err.context("Operation failed"))
            }
        }
    }
    ```

3.  **Moving `Copy` types into tasks**:
    - When spawning tasks, `Copy` types are prevented from being moved into the task async block. Use `ctx::NoCopy` to wrap `Copy` types when you want to move them inside tasks, then unwrap them inside the task.

    ```rust
    use zksync_concurrency::{ctx, scope};

    let epoch = 42u64; // A Copy type
    
    // Without NoCopy - this won't compile
    scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            println!("Task processing epoch: {}", epoch);
            Ok(())
        });
        
        Ok(())
    }).await.unwrap();

    // With NoCopy - epoch is moved, ensuring transfer of ownership
    let nc_epoch = ctx::NoCopy(epoch);
    scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            let epoch = nc_epoch.into(); // epoch is moved
            println!("Task processing epoch: {}", epoch);
            Ok(())
        });
        // nc_epoch is no longer accessible here - it was moved
        Ok(())
    }).await.unwrap();
    ```

## Utilities

The `zksync_concurrency` crate also provides several context-aware utilities:

- **`ctx::channel`**: Bounded and unbounded MPSC (multi-producer, single-consumer) channels. `send()` and `recv()` operations are context-aware and will return `Err(ctx::Canceled)` if the context is canceled during the operation. Channel disconnection is generally not directly observable; tasks should rely on context cancellation for termination.
- **`oneshot`**: Context-aware one-shot channels, for sending a single value between two tasks.
- **`limiter`**: A rate limiter that supports delayed permit consumption. `acquire()` is context-aware.
- **`time` Module & `ctx::Clock`**:
    - `time::Duration`, `time::Instant`, `time::Utc`, `time::Deadline`.
    - `ctx::Clock` (accessible via `ctx.clock()`, `ctx.now()`, `ctx.now_utc()`, `ctx.sleep()`) provides an abstraction over time sources. This is particularly useful for testing, where a `ManualClock` can be used to control time progression deterministically.
- **`io` Module**: Context-aware asynchronous I/O operations (e.g., `read`, `read_exact`, `write_all`) built on top of `tokio::io`. These operations will honor context cancellation.
- **`net` Module**: Context-aware network utilities, such as `Host::resolve()` for DNS resolution and `tcp::connect`/`tcp::accept` for TCP operations, all respecting context cancellation.
- **`sync` Module**: Context-aware synchronization primitives like `Mutex` (`sync::lock`), `Semaphore` (`sync::acquire`), and `Notify` (`sync::notified`). It also includes a `prunable_mpsc` channel where messages can be filtered or pruned based on predicates.
- **`signal::Once`**: A simple signal that can be sent once and awaited by multiple tasks, also context-aware.

## Best Practices

1.  **Pass `&ctx::Ctx` Everywhere**: Any function that might block, perform I/O, or is part of a larger cancellable operation should accept a `&ctx::Ctx` as its first argument.
2.  **Favor `ctx.wait()`**: When `await`ing a future that is not itself context-aware, wrap it with `ctx.wait(your_future).await?` to make it responsive to cancellation.
3.  **Check `ctx.is_active()` in Loops**: In long-running computations or loops, periodically check `ctx.is_active()` and return `Err(ctx::Canceled)` or clean up and exit if it's false.
4.  **Use `scope` for Related Tasks**: Group related concurrent tasks within a `scope` to ensure proper lifecycle management and error propagation.
5.  **Return `Result<T, YourError>` from Tasks**: Tasks spawned in a scope should return a `Result`. The error type `YourError` should be consistent across the scope (often `anyhow::Error` or `ctx::Error`).
6.  **Keep Tasks Focused**: Tasks should ideally represent a single logical unit of work.
7.  **Resource Cleanup**: When a task detects cancellation, it should clean up any resources it holds before returning.