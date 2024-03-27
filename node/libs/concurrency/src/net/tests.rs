use super::*;

#[tokio::test]
async fn test_resolve() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let addrs = Host("localhost:1234".into())
        .resolve(ctx)
        .await
        .unwrap()
        .unwrap();
    // We assume here that loopback host "localhost" is always configured here.
    // If that turns out to be problematic we should use a more robust DNS resulution
    // library, so that we can run our own DNS server in tests.
    assert!(!addrs.is_empty());
    for addr in addrs {
        // Verify the port and that the returned address is actually a loopback.
        assert_eq!(addr.port(), 1234);
        assert!(addr.ip().is_loopback());

        // Verify that resolving the serialized address returns the same address.
        let got = Host(addr.to_string()).resolve(ctx).await.unwrap().unwrap();
        assert!(got.len() == 1);
        assert_eq!(got[0], addr);
    }

    // Host without port should not resolve.
    assert!(Host("localhost".into())
        .resolve(ctx)
        .await
        .unwrap()
        .is_err());
    // Invalid host name should not resolve.
    assert!(Host("$#$:6746".into()).resolve(ctx).await.unwrap().is_err());
    // Empty host name should not resolve.
    assert!(Host(String::new()).resolve(ctx).await.unwrap().is_err());
}
