use ark_std::{UniformRand, rand::Rng};
use hints::{
    snark::{F, GlobalData, Hint, KZG, finish_setup},
    *,
};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        // fn sample_weights(n: usize, rng: &mut impl Rng) -> Vec<F> {
        //     (0..n).map(|_| F::from(u64::rand(rng))).collect()
        // }

        // Generate random ("insecure") KZG setup
        let domain = 1 << 3; // Maximum number of signers
        let gd = hints::setup_eth(domain).expect("Setup failed");
        let n = 7;

        // Generate keys for each participant
        let mut rng = ark_std::test_rng();
        let sk: Vec<SecretKey> = (0..n).map(|_| SecretKey::random(&mut rng)).collect();
        let pks: Vec<PublicKey> = sk.iter().map(|sk| sk.public(&gd)).collect();
        
        println!("{pks:?}");

        // Generate hints for each participant
        let hints: Vec<Hint> = sk
            .iter()
            .enumerate()
            .map(|(i, sk)| snark::hintgen(&gd, sk, domain, i).expect("Failed to generate hints"))
            .collect();

        // Setup with weights
        // let weights = sample_weights(n, &mut rng);
        let weights: Vec<F> = vec![1u64,1,1,1,1,1,1].into_iter().map(|x| F::from(x)).collect();
        let hints::snark::SetupResult {agg_key, vk, party_errors} = finish_setup(&gd, domain, pks, &hints, weights.clone())
            .expect("Failed to finish setup");

        // Sign a message with each signer
        let partials: Vec<(usize, PartialSignature)> = sk
            .iter()
            .enumerate()
            .map(|(i, sk)| (i, sk.sign(b"hello")))
            .collect();

        // Aggregate signatures
        let sig1 = agg_key
            .aggregate(&gd, F::from(5), &partials, weights.clone(), b"hello")
            .unwrap();
        println!("{sig1:?}\n");

        // Verify the aggregated signature
        assert!(sig1.verify(&vk, b"hello").unwrap());

        let sig2 = agg_key
            .aggregate(&gd, F::from(5), &partials[..5], weights.clone(), b"hello")
            .unwrap();
        println!("{sig2:?}\n");
        assert!(sig2.verify(&vk, b"hello").unwrap());

        let sig3 = agg_key
            .aggregate(&gd, F::from(5), &partials[2..], weights.clone(), b"hello")
            .unwrap();
        println!("{sig3:?}\n");
        assert!(sig3.verify(&vk, b"hello").unwrap());
    }
}
