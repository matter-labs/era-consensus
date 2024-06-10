//! This is a wrapper around channels to make it simpler and less error-prone to connect actors and the dispatcher.
//! A Pipe is a basically a bi-directional unbounded channel.

use std::future::Future;
use zksync_concurrency::ctx::{self, channel, Ctx};

/// This is the end of the Pipe that should be held by the actor.
///
/// The actor can receive `In` and send back `Out` messages.
pub type ActorPipe<In, Out> = Pipe<In, Out>;

/// This is the end of the Pipe that should be held by the dispatcher.
///
/// The dispatcher can send `In` messages and receive `Out` back.
pub type DispatcherPipe<In, Out> = Pipe<Out, In>;

/// This is a generic Pipe end.
///
/// It is used to receive `In` and send `Out` messages.
///
/// When viewed from the perspective of [new]:
/// * messages of type `In` are going right-to-left
/// * messages of type `Out` are going left-to-right
///
/// ```text
///  In  <- -------- <- In
///         | Pipe |
///  Out -> -------- -> Out
///  ^                  ^
///  Actor              Dispatcher
/// ```
#[derive(Debug)]
pub struct Pipe<In, Out> {
    /// This is the channel that receives messages.
    pub recv: channel::UnboundedReceiver<In>,
    /// This is the channel that sends messages.
    pub send: channel::UnboundedSender<Out>,
}

impl<In, Out> Pipe<In, Out> {
    /// Sends a message to the pipe.
    pub fn send(&self, msg: Out) {
        self.send.send(msg)
    }

    /// Awaits a message from the pipe.
    pub fn recv<'a>(
        &'a mut self,
        ctx: &'a Ctx,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<In>>> {
        self.recv.recv(ctx)
    }

    /// Tries to get a message from the pipe. Will return None if the pipe is empty.
    pub fn try_recv(&mut self) -> Option<In> {
        self.recv.try_recv()
    }
}

/// This function creates a new Pipe. It returns the two ends of the pipe, for the actor and the dispatcher.
pub fn new<In, Out>() -> (ActorPipe<In, Out>, DispatcherPipe<In, Out>) {
    let (input_sender, input_receiver) = channel::unbounded();
    let (output_sender, output_receiver) = channel::unbounded();

    let pipe_actor = Pipe {
        recv: input_receiver,
        send: output_sender,
    };

    let pipe_dispatcher = Pipe {
        recv: output_receiver,
        send: input_sender,
    };

    (pipe_actor, pipe_dispatcher)
}
