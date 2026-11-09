//! Optional network check; excluded from ordinary deterministic builds/tests.
use p2pshare_control_iroh as control;
#[tokio::test]
#[ignore = "Requires public Iroh relay access"]
async fn public_relay_bootstrap_encrypted_control_roundtrip() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.build/public-control-tests")
        .join(format!("{}-{}", std::process::id(), rand::random::<u32>()));
    let host = control::bind(&root.join("host.key"), false).await?;
    let guest = control::bind(&root.join("guest.key"), false).await?;
    let mut invite = control::Invite::new(&host);
    invite.coordinator.addrs.retain(|addr| addr.is_relay());
    anyhow::ensure!(
        !invite.coordinator.addrs.is_empty(),
        "No public relay online"
    );
    let server = async {
        let incoming = host
            .accept()
            .await
            .ok_or_else(|| anyhow::anyhow!("Endpoint closed"))?;
        let connection = incoming.await?;
        anyhow::ensure!(
            control::peer_id(&connection) == guest.id().to_string(),
            "Authenticated identity mismatch"
        );
        let (mut send, mut recv) = connection.accept_bi().await?;
        let message: String = control::receive(&mut recv).await?;
        anyhow::ensure!(
            message == "P2PShare public control test",
            "Encrypted message mismatch"
        );
        control::send(&mut send, &"ack").await?;
        Ok::<_, anyhow::Error>(connection)
    };
    let client = async {
        let connection = control::connect(&guest, &invite).await?;
        let (mut send, mut recv) = connection.open_bi().await?;
        control::send(&mut send, &"P2PShare public control test").await?;
        let ack: String = control::receive(&mut recv).await?;
        anyhow::ensure!(ack == "ack", "Encrypted reply mismatch");
        Ok::<_, anyhow::Error>(connection)
    };
    let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::try_join!(server, client)
    })
    .await??;
    a.close(0u32.into(), b"Test complete");
    b.close(0u32.into(), b"Test complete");
    host.close().await;
    guest.close().await;
    println!(
        "Public-relay invitation bootstrap and authenticated encrypted control roundtrip passed"
    );
    Ok(())
}
