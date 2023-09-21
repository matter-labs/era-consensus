use crate::noise;
use concurrency::{ctx, net, scope};

pub(crate) async fn pipe(ctx: &ctx::Ctx) -> (noise::Stream, noise::Stream) {
    scope::run!(ctx, |ctx, s| async {
        let (s1, s2) = net::tcp::testonly::pipe(ctx).await;
        let s1 = s.spawn(async { noise::Stream::client_handshake(ctx, s1).await });
        let s2 = s.spawn(async { noise::Stream::server_handshake(ctx, s2).await });
        Ok((s1.join(ctx).await?, s2.join(ctx).await?))
    })
    .await
    .unwrap()
}
