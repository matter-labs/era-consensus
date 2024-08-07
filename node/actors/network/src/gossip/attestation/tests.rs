#![allow(dead_code)]
use super::*;
use zksync_concurrency::testonly::abort_on_panic;
use rand::Rng as _;

type Vote = Arc<attester::Signed<attester::Batch>>;

#[derive(Default, Debug, PartialEq)]
struct Votes(im::HashMap<attester::PublicKey, Vote>);

impl Votes {
    fn insert(&mut self, vote: Vote) {
        self.0.insert(vote.key.clone(), vote);
    }

    fn get(&mut self, key: &attester::SecretKey) -> Vote {
        self.0.get(&key.public()).unwrap().clone()
    }
}

impl StateReceiver {
    fn snapshot(&self) -> Votes {
        let Some(state) = self.recv.borrow().clone() else { return Votes::default() };
        state.votes.values().cloned().into()
    }
}

impl<T: IntoIterator<Item=Vote>> From<T> for Votes {
    fn from(x: T) -> Self {
        Self(x.into_iter().map(|v|(v.key.clone(),v)).collect())
    }
}

#[tokio::test]
async fn test_insert_votes() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock); 
    let rng = &mut ctx.rng();

    let state = StateWatch::new(None);
    let genesis : attester::GenesisHash = rng.gen();
    let first : attester::BatchNumber = rng.gen();
    for i in 0..3 {
        let keys: Vec<attester::SecretKey> = (0..8).map(|_| rng.gen()).collect();
        let config = Config {
            batch_to_attest: attester::Batch {
                genesis,
                number: first + i,
                hash: rng.gen(),
            },
            committee: attester::Committee::new(keys.iter().map(|k| attester::WeightedAttester {
                key: k.public(),
                weight: 1250,
            })).unwrap(),
        };
        state.update_config(config.clone()).await.unwrap();
        assert_eq!(Votes::from([]), state.votes().into());
        let mut recv = state.subscribe();

        let all_votes : Vec<Vote> = keys.iter().map(|k| k.sign_msg(config.batch_to_attest.clone().into()).into()).collect();

        // Initial votes.
        state.insert_votes(all_votes[0..3].iter().cloned()).await.unwrap();
        assert_eq!(Votes::from(all_votes[0..3].iter().cloned()), state.votes().into());
        assert_eq!(Votes::from(all_votes[0..3].iter().cloned()), recv.wait_for_new_votes(ctx).await.unwrap().into());

        // Adding votes gradually.
        state.insert_votes(all_votes[3..5].iter().cloned()).await.unwrap();
        state.insert_votes(all_votes[5..7].iter().cloned()).await.unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), state.votes().into());
        assert_eq!(Votes::from(all_votes[3..7].iter().cloned()), recv.wait_for_new_votes(ctx).await.unwrap().into());

        // Readding already inserded votes (noop).
        state.insert_votes(all_votes[2..6].iter().cloned()).await.unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), state.votes().into());

        // Adding votes out of committee (noop).
        state.insert_votes((0..3).map(|_| {
            let k : attester::SecretKey = rng.gen();
            k.sign_msg(config.batch_to_attest.clone()).into()
        })).await.unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), state.votes().into());

        // Adding votes for different batch (noop).
        state.insert_votes((0..3).map(|_| {
            let k : attester::SecretKey = rng.gen();
            k.sign_msg(attester::Batch {
                genesis: config.batch_to_attest.genesis,
                number: rng.gen(),
                hash: rng.gen(),
            }).into()
        })).await.unwrap();
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), state.votes().into());

        // Adding incorrect votes (error).
        let mut bad_vote = (*all_votes[7]).clone();
        bad_vote.sig = rng.gen();
        assert!(state.insert_votes([bad_vote.into()].into_iter()).await.is_err());
        assert_eq!(Votes::from(all_votes[0..7].iter().cloned()), state.votes().into());
    
        // Add the last vote mixed with already added votes.
        state.insert_votes(all_votes[5..].iter().cloned()).await.unwrap();
        assert_eq!(Votes::from(all_votes.clone()), state.votes().into());
        assert_eq!(Votes::from(all_votes[7..].iter().cloned()), recv.wait_for_new_votes(ctx).await.unwrap().into());
    }
}

#[tokio::test]
fn test_wait_for_qc() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock); 
    let rng = &mut ctx.rng();

    let state = StateWatch::new(None);
    let genesis : attester::GenesisHash = rng.gen();
    let first : attester::BatchNumber = rng.gen();

    for i in 0..10 {
        let committee_size = rng.gen_range(1..20);
        let keys: Vec<attester::SecretKey> = (0..committee_size).map(|_| rng.gen()).collect();
        let config = Config {
            batch_to_attest: attester::Batch {
                genesis,
                number: first + i,
                hash: rng.gen(),
            },
            committee: attester::Committee::new(keys.iter().map(|k| attester::WeightedAttester {
                key: k.public(),
                weight: rng.gen_range(1..=100),
            })).unwrap(),
        };
        state.update_config(config.clone()).await.unwrap();
        // TODO: add votes gradually, extract qc each time and verify the qc.
        for k in &keys {
            // We need 4/5+1 for quorum, so let's say ~80% vote on the second batch.
            let b = usize::from(rng.gen_range(0..100) < 80);
            let batch = &batches[b].0;
            let vote = sk.sign_msg(batch.clone());
            votes
                .update(&attesters, &genesis, &[Arc::new(vote)])
                .unwrap();
            batches[b].1 += attesters.weight(&sk.public()).unwrap();

            // Check that as soon as we have quorum it's found.
            if batches[b].1 >= attesters.threshold() {
                let qc = votes
                    .find_quorum(&attesters, &genesis)
                    .expect("should find quorum");
                assert!(qc.message == *batch);
                assert!(qc.signatures.keys().count() > 0);
            }
        }

        // Check that if there was no quoroum then we don't find any.
        if !batches.iter().any(|b| b.1 >= attesters.threshold()) {
            assert!(votes.find_quorum(&attesters, &genesis).is_none());
        }

        // Check that the minimum batch number prunes data.
        let last_batch = batches[1].0.number;

        votes.set_min_batch_number(last_batch);
        assert!(votes.votes.values().all(|v| v.msg.number >= last_batch));
        assert!(votes.support.keys().all(|n| *n >= last_batch));

        votes.set_min_batch_number(last_batch.next());
        assert!(votes.votes.is_empty());
        assert!(votes.support.is_empty());
    }
}
