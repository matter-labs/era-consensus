use super::*;
use rand::{seq::SliceRandom as _, Rng as _};
use zksync_concurrency::testonly::abort_on_panic;

type Vote = Arc<attester::Signed<attester::Batch>>;

#[derive(Default, Debug, PartialEq)]
struct Votes(im::HashMap<attester::PublicKey, Vote>);

impl<T: IntoIterator<Item = Vote>> From<T> for Votes {
    fn from(x: T) -> Self {
        Self(x.into_iter().map(|v| (v.key.clone(), v)).collect())
    }
}

#[tokio::test]
async fn test_insert_votes() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let ctrl = Controller::new(None);
    let genesis: attester::GenesisHash = rng.gen();
    let first: attester::BatchNumber = rng.gen();
    for i in 0..3 {
        tracing::info!("iteration {i}");
        let keys: Vec<attester::SecretKey> = (0..8).map(|_| rng.gen()).collect();
        let config = Arc::new(Config {
            batch_to_attest: attester::Batch {
                genesis,
                number: first + i,
                hash: rng.gen(),
            },
            committee: attester::Committee::new(keys.iter().map(|k| attester::WeightedAttester {
                key: k.public(),
                weight: 1250,
            }))
            .unwrap()
            .into(),
        });
        let ctrl_votes = || Votes::from(ctrl.votes(&config.batch_to_attest));
        ctrl.update_config(config.clone()).await.unwrap();
        assert_eq!(Votes::from([]), ctrl_votes());
        let mut recv = ctrl.subscribe();
        let diff = recv.wait_for_diff(ctx).await.unwrap();
        assert_eq!(diff.config.as_ref(), Some(&config));
        assert_eq!(Votes::default(), diff.votes.into());

        let all_votes: Vec<Vote> = keys
            .iter()
            .map(|k| k.sign_msg(config.batch_to_attest.clone()).into())
            .collect();

        tracing::info!("Initial votes.");
        ctrl.insert_votes(all_votes[0..3].iter().cloned())
            .await
            .unwrap();
        assert_eq!(Votes::from(all_votes[0..3].iter().cloned()), ctrl_votes());
        let diff = recv.wait_for_diff(ctx).await.unwrap();
        assert!(diff.config.is_none());
        assert_eq!(
            Votes::from(all_votes[0..3].iter().cloned()),
            diff.votes.into()
        );

        tracing::info!("Adding votes gradually.");
        ctrl.insert_votes(all_votes[3..5].iter().cloned())
            .await
            .unwrap();
        ctrl.insert_votes(all_votes[5..7].iter().cloned())
            .await
            .unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), ctrl_votes());
        let diff = recv.wait_for_diff(ctx).await.unwrap();
        assert!(diff.config.is_none());
        assert_eq!(
            Votes::from(all_votes[3..7].iter().cloned()),
            diff.votes.into()
        );

        tracing::info!("Readding already inserded votes (noop).");
        ctrl.insert_votes(all_votes[2..6].iter().cloned())
            .await
            .unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), ctrl_votes());

        tracing::info!("Adding votes out of committee (error).");
        assert!(ctrl
            .insert_votes((0..3).map(|_| {
                let k: attester::SecretKey = rng.gen();
                k.sign_msg(config.batch_to_attest.clone()).into()
            }))
            .await
            .is_err());
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), ctrl_votes());

        tracing::info!("Adding votes for different batch (noop).");
        ctrl.insert_votes((0..3).map(|_| {
            let k: attester::SecretKey = rng.gen();
            k.sign_msg(attester::Batch {
                genesis: config.batch_to_attest.genesis,
                number: rng.gen(),
                hash: rng.gen(),
            })
            .into()
        }))
        .await
        .unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), ctrl_votes());

        tracing::info!("Adding incorrect votes (error).");
        let mut bad_vote = (*all_votes[7]).clone();
        bad_vote.sig = rng.gen();
        assert!(ctrl
            .insert_votes([bad_vote.into()].into_iter())
            .await
            .is_err());
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), ctrl_votes());

        tracing::info!("Add the last vote mixed with already added votes.");
        ctrl.insert_votes(all_votes[5..].iter().cloned())
            .await
            .unwrap();
        assert_eq!(Votes::from(all_votes.clone()), ctrl_votes());
        let diff = recv.wait_for_diff(ctx).await.unwrap();
        assert!(diff.config.is_none());
        assert_eq!(
            Votes::from(all_votes[7..].iter().cloned()),
            diff.votes.into()
        );
    }
}

#[tokio::test]
async fn test_wait_for_qc() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let ctrl = Controller::new(None);
    let genesis: attester::GenesisHash = rng.gen();
    let first: attester::BatchNumber = rng.gen();

    for i in 0..10 {
        tracing::info!("iteration {i}");
        let committee_size = rng.gen_range(1..20);
        let keys: Vec<attester::SecretKey> = (0..committee_size).map(|_| rng.gen()).collect();
        let config = Arc::new(Config {
            batch_to_attest: attester::Batch {
                genesis,
                number: first + i,
                hash: rng.gen(),
            },
            committee: attester::Committee::new(keys.iter().map(|k| attester::WeightedAttester {
                key: k.public(),
                weight: rng.gen_range(1..=100),
            }))
            .unwrap()
            .into(),
        });
        let mut all_votes: Vec<Vote> = keys
            .iter()
            .map(|k| k.sign_msg(config.batch_to_attest.clone()).into())
            .collect();
        all_votes.shuffle(rng);
        ctrl.update_config(config.clone()).await.unwrap();
        loop {
            let end = rng.gen_range(0..=committee_size);
            tracing::info!("end = {end}");
            ctrl.insert_votes(all_votes[..end].iter().cloned())
                .await
                .unwrap();
            // Waiting for the previous qc should immediately return None.
            assert_eq!(
                None,
                ctrl.wait_for_qc(ctx, config.batch_to_attest.number.prev().unwrap())
                    .await
                    .unwrap()
            );
            if config
                .committee
                .weight_of_keys(all_votes[..end].iter().map(|v| &v.key))
                >= config.committee.threshold()
            {
                let qc = ctrl
                    .wait_for_qc(ctx, config.batch_to_attest.number)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(qc.message, config.batch_to_attest);
                qc.verify(genesis, &config.committee).unwrap();
                break;
            }
            assert_eq!(None, ctrl.state.subscribe().borrow().as_ref().unwrap().qc());
        }
    }
}
