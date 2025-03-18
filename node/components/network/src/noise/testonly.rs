use zksync_concurrency::{ctx, scope};

use crate::{metrics, noise};

pub(crate) async fn pipe(ctx: &ctx::Ctx) -> (noise::Stream, noise::Stream) {
    scope::run!(ctx, |ctx, s| async {
        let (outbound_stream, inbound_stream) = metrics::MeteredStream::test_pipe(ctx).await;
        let outbound_task =
            s.spawn(async { noise::Stream::client_handshake(ctx, outbound_stream).await });
        let inbound_task =
            s.spawn(async { noise::Stream::server_handshake(ctx, inbound_stream).await });
        Ok((
            outbound_task.join(ctx).await?,
            inbound_task.join(ctx).await?,
        ))
    })
    .await
    .unwrap()
}
