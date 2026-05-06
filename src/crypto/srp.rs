use crate::core::error::CryptoError;
use num_bigint::BigUint;
use sha2::{Digest, Sha512};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

const N_BYTES: usize = 384;

const RFC5054_N_3072: &str = concat!(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E08",
    "8A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B",
    "302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9",
    "A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE6",
    "49286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8",
    "FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D",
    "670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C",
    "180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
    "3995497CEA956AE515D2261898FA051015728E5A8AAAC42DAD33170D",
    "04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7D",
    "B3970F85A6E1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D226",
    "1AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B18177B200C",
    "BBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFC",
    "E0FD108E4B82D120A93AD2CAFFFFFFFFFFFFFFFF"
);

pub struct SrpParams {
    pub n: BigUint,
    pub g: BigUint,
}

impl Default for SrpParams {
    fn default() -> Self {
        let n = BigUint::parse_bytes(RFC5054_N_3072.as_bytes(), 16)
            .expect("Invalid RFC 5054 prime constant");
        let g = BigUint::from(5u32);
        Self { n, g }
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SrpClient {
    #[zeroize(skip)]
    params: SrpParams,
    identity: Vec<u8>,
    password: Vec<u8>,
    private_key: Vec<u8>,
    #[zeroize(skip)]
    public_key: BigUint,
}

pub struct SrpChallenge {
    pub salt: [u8; 16],
    pub server_public_key: Vec<u8>,
}

pub struct SrpProof {
    pub client_proof: Vec<u8>,
    pub shared_secret: Vec<u8>,
    pub expected_server_proof: Vec<u8>,
}

impl SrpClient {
    pub fn new(identity: &[u8], password: &[u8]) -> Self {
        let params = SrpParams::default();

        let a = loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).expect("operating system random source failed");

            let candidate = BigUint::from_bytes_be(&bytes);
            if candidate != BigUint::ZERO {
                break candidate;
            }
        };
        let private_key = a.to_bytes_be();

        let public_key = params.g.modpow(&a, &params.n);

        Self {
            params,
            identity: identity.to_vec(),
            password: password.to_vec(),
            private_key,
            public_key,
        }
    }

    pub fn public_key(&self) -> Vec<u8> {
        pad_to_n(&self.public_key)
    }

    pub fn process_challenge(&self, challenge: &SrpChallenge) -> Result<SrpProof, CryptoError> {
        let b = BigUint::from_bytes_be(&challenge.server_public_key);

        if &b % &self.params.n == BigUint::ZERO {
            return Err(CryptoError::Encryption(
                "Invalid server public key: B mod N = 0".to_string(),
            ));
        }

        let a = BigUint::from_bytes_be(&self.private_key);

        let u = compute_u(&self.public_key, &b, &self.params);
        if u == BigUint::ZERO {
            return Err(CryptoError::Encryption(
                "Invalid u value: u = 0".to_string(),
            ));
        }

        let x = compute_x(&challenge.salt, &self.identity, &self.password);

        let k = compute_k(&self.params);

        let g_x = self.params.g.modpow(&x, &self.params.n);
        let k_gx = (&k * &g_x) % &self.params.n;

        let base = if b >= k_gx {
            (&b - &k_gx) % &self.params.n
        } else {
            (&b + &self.params.n - &k_gx) % &self.params.n
        };

        let exponent = (&a + &u * &x) % (&self.params.n - BigUint::from(1u32));
        let s = base.modpow(&exponent, &self.params.n);

        let s_padded = pad_to_n(&s);
        let mut hasher = Sha512::new();
        hasher.update(&s_padded);
        let shared_secret = hasher.finalize().to_vec();

        let client_proof = compute_m1(
            &self.params,
            &self.identity,
            &challenge.salt,
            &self.public_key,
            &b,
            &shared_secret,
        );

        let a_padded = pad_to_n(&self.public_key);
        let mut hasher = Sha512::new();
        hasher.update(&a_padded);
        hasher.update(&client_proof);
        hasher.update(&shared_secret);
        let expected_server_proof = hasher.finalize().to_vec();

        Ok(SrpProof {
            client_proof,
            shared_secret,
            expected_server_proof,
        })
    }

    pub fn verify_server_proof(&self, proof: &[u8], expected: &[u8]) -> bool {
        proof.ct_eq(expected).into()
    }
}

fn compute_m1(
    params: &SrpParams,
    identity: &[u8],
    salt: &[u8],
    a: &BigUint,
    b: &BigUint,
    k: &[u8],
) -> Vec<u8> {
    let n_bytes = pad_to_n(&params.n);
    let mut hasher = Sha512::new();
    hasher.update(&n_bytes);
    let h_n = hasher.finalize();

    let g_bytes = params.g.to_bytes_be();
    let mut hasher = Sha512::new();
    hasher.update(&g_bytes);
    let h_g = hasher.finalize();

    let mut xor_result = [0u8; 64];
    for i in 0..64 {
        xor_result[i] = h_n[i] ^ h_g[i];
    }

    let mut hasher = Sha512::new();
    hasher.update(identity);
    let h_i = hasher.finalize();

    let mut hasher = Sha512::new();
    hasher.update(xor_result);
    hasher.update(h_i);
    hasher.update(salt);
    hasher.update(pad_to_n(a));
    hasher.update(pad_to_n(b));
    hasher.update(k);
    hasher.finalize().to_vec()
}

fn pad_to_n(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    if bytes.len() >= N_BYTES {
        bytes[bytes.len() - N_BYTES..].to_vec()
    } else {
        let mut padded = vec![0u8; N_BYTES - bytes.len()];
        padded.extend_from_slice(&bytes);
        padded
    }
}

fn compute_k(params: &SrpParams) -> BigUint {
    let n_bytes = pad_to_n(&params.n);
    let g_bytes = pad_to_n(&params.g);

    let mut hasher = Sha512::new();
    hasher.update(&n_bytes);
    hasher.update(&g_bytes);
    let hash = hasher.finalize();

    BigUint::from_bytes_be(&hash)
}

fn compute_u(a: &BigUint, b: &BigUint, _params: &SrpParams) -> BigUint {
    let a_bytes = pad_to_n(a);
    let b_bytes = pad_to_n(b);

    let mut hasher = Sha512::new();
    hasher.update(&a_bytes);
    hasher.update(&b_bytes);
    let hash = hasher.finalize();

    BigUint::from_bytes_be(&hash)
}

fn compute_x(salt: &[u8], identity: &[u8], password: &[u8]) -> BigUint {
    let mut hasher = Sha512::new();
    hasher.update(identity);
    hasher.update(b":");
    hasher.update(password);
    let inner_hash = hasher.finalize();

    let mut hasher = Sha512::new();
    hasher.update(salt);
    hasher.update(inner_hash);
    let hash = hasher.finalize();

    BigUint::from_bytes_be(&hash)
}
