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
        tokio::time::sleep(time::Duration::seconds(1)).await;
        "done"
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
```

### 2. `scope` - Structured Concurrency

Structured concurrency ensures that concurrent tasks are managed within a well-defined scope. If a scope exits, all tasks spawned within it are guaranteed to have completed or been canceled.

- **`scope::run!` (async) and `scope::run_blocking!` (sync)**: These macros create a new scope. The scope manages a group of tasks.
- **Task Spawning**:
    - **Main Tasks**: `s.spawn(async_fn)` or `s.spawn_blocking(sync_fn)`. The scope waits for all main tasks to complete. If any main task returns an error, the scope is canceled.
    - **Background Tasks**: `s.spawn_bg(async_fn)` or `s.spawn_bg_blocking(sync_fn)`. These tasks can continue running even after all main tasks complete, but they are also canceled when the scope is canceled. They are useful for tasks like request handlers or periodic jobs that don't define the primary completion of the scope.
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

async fn application_logic(global_ctx: &ctx::Ctx) -> Result<String, anyhow::Error> {
    let counter = AtomicUsize::new(0);

    // scope::run! takes the parent context and a closure.
    // The closure receives the scope's context and a reference to the scope itself.
    let result_from_scope: Result<String, anyhow::Error> = scope::run!(global_ctx, |ctx, s| async {
        // Spawn a main async task
        s.spawn(async {
            ctx.sleep(time::Duration::milliseconds(50)).await?;
            counter.fetch_add(1, Ordering::Relaxed);
            println!("Main async task 1 completed.");
            // Tasks should return Result<T, E> where E is the scope's error type
            Ok(())
        });

        // Spawn another main blocking task
        let handle = s.spawn_blocking(|| {
            // Simulate work
            std::thread::sleep(std::time::Duration::from_millis(100));
            counter.fetch_add(1, Ordering::Relaxed);
            println!("Main blocking task completed.");

            // Example of returning an error:
            // return Err(anyhow::anyhow!("Something went wrong in blocking task"));
            Ok(100) // This task returns Ok(100)
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
        // For example, to make the scope fail if the counter is unexpected:
        // if counter.load(Ordering::Relaxed) != 2 {
        //     return Err(anyhow::anyhow!("Counter mismatch"));
        // }
        Ok("All main tasks launched".to_string())
    }).await;

    println!("Scope finished. Final counter: {}", counter.load(Ordering::Relaxed));
    result_from_scope
}
```

### Error Types

- **`ctx::Canceled`**: A distinct error type indicating that an operation was canceled because its context was canceled.
- **`ctx::Error`**: An enum that can be either `ctx::Canceled` or `anyhow::Error`. This is useful for functions that can fail either due to cancellation or other reasons.
  ```rust
  use zksync_concurrency::ctx;

  fn operation_that_can_be_canceled(ctx: &ctx::Ctx) -> ctx::Result<String> {
      if !ctx.is_active() {
          return Err(ctx::Canceled.into());
      }
      // ... some fallible operation
      // let data = might_fail().map_err(anyhow::Error::from)?;
      // Ok(data)
      ctx::Ok("Success!".to_string()) // Equivalent to Ok(Ok(...)) for ctx::Result
  }
  ```
- **`error::Wrap`**: A trait similar to `anyhow::Context` for adding context to your custom error types that might embed `anyhow::Error`, while correctly handling `ctx::Canceled`.

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
7.  **Resource Cleanup**: When a task detects cancellation (e.g., `ctx.wait()` returns `Err(ctx::Canceled)` or `ctx.is_active()` is false), it should clean up any resources it holds before returning.

## Full Example

```rust
use zksync_concurrency::{ctx, scope, time, error::Wrap};
use std::sync::atomic::{AtomicBool, Ordering};

// Custom error for our application
#[derive(Debug, thiserror::Error)]
enum MyError {
    #[error("Operation failed: {0}")]
    OperationFailed(String),
    #[error(transparent)]
    Context(#[from] ctx::Error), // Allow easy conversion from ctx::Error
}

// A worker function
async fn worker_task(ctx: &ctx::Ctx, id: usize, fail_flag: &AtomicBool) -> Result<usize, MyError> {
    println!("[Worker {}] Starting", id);
    for i in 0..5 {
        if !ctx.is_active() {
            println!("[Worker {}] Canceled during iteration {}", id, i);
            return Err(ctx::Canceled.into()).wrap(|| format!("Worker {} canceled", id));
        }
        println!("[Worker {}] Processing item {}", id, i);
        ctx.sleep(time::Duration::milliseconds(50 + (id * 20) as i64)).await
            .map_err(ctx::Error::from)?; // Convert Canceled to ctx::Error

        if id == 1 && i == 2 && fail_flag.load(Ordering::Relaxed) {
            println!("[Worker {}] Simulating failure", id);
            return Err(MyError::OperationFailed(format!("Worker {} failed intentionally", id)));
        }
    }
    println!("[Worker {}] Finished successfully", id);
    Ok(id * 10)
}

// Main application entry point using the concurrency primitives
async fn run_application(should_fail: bool) -> Result<Vec<usize>, MyError> {
    let root_ctx = ctx::root(); // Create a root context
    let fail_flag = AtomicBool::new(should_fail);

    println!("Application starting. Simulating failure: {}", should_fail);

    let results = scope::run!(&root_ctx, |ctx, s| async {
        let mut handles = vec![];
        for i in 0..3 {
            let handle = s.spawn(worker_task(ctx, i, &fail_flag));
            handles.push(handle);
        }

        // Background task example
        s.spawn_bg(async {
            loop {
                if !ctx.is_active() {
                    println!("[Monitor] Scope canceled, monitor exiting.");
                    break;
                }
                println!("[Monitor] System nominal.");
                if ctx.sleep(time::Duration::seconds(1)).await.is_err() {
                    println!("[Monitor] Sleep canceled, monitor exiting.");
                    break;
                }
            }
            Ok(()) // Background tasks also return Result
        });

        let mut collected_results = vec![];
        for (idx, handle) in handles.into_iter().enumerate() {
            match handle.join(ctx).await {
                Ok(res) => {
                    println!("[Main] Worker {} joined with result: {}", idx, res);
                    collected_results.push(res);
                }
                Err(ctx::Canceled) => {
                    // This occurs if the scope was canceled due to another task's error,
                    // or if this specific task returned an error that canceled the scope.
                    println!("[Main] Worker {} was canceled or its scope failed.", idx);
                    // Depending on requirements, you might want to propagate a generic error
                    // or specific information. Here, the scope::run! will return the first
                    // actual error that occurred.
                }
            }
        }
        // The root task of the scope.
        // If this returns an error, the scope::run! will return it (if no other task failed first).
        Ok(collected_results)
    }).await?;

    println!("Application finished. Collected results: {:?}", results);
    Ok(results)
}

// Example of how to run this (e.g., in a #[tokio::main] function)
// async fn main() {
//     match run_application(true).await {
//         Ok(results) => println!("Success: {:?}", results),
//         Err(e) => println!("Error: {:?}", e),
//     }
//     println!("---");
//     match run_application(false).await {
//         Ok(results) => println!("Success: {:?}", results),
//         Err(e) => println!("Error: {:?}", e),
//     }
// }
```

This README provides a good overview of the `zksync_concurrency` crate, its main components, and how to use them effectively for managing concurrent operations in Rust.
