//! Prime generation and finite-field Diffie-Hellman for node:crypto.

use crypto_bigint::modular::{BoxedMontyForm, BoxedMontyParams};
use crypto_bigint::{BoxedUint, NonZero, Odd, RandomBits, RandomMod, Resize};
use crypto_primes::{Flavor, is_prime, random_prime};

use super::keys::{bytes, integer};
use super::rng;

pub(super) fn check_prime(candidate: &[u8]) -> bool {
    let candidate = integer(candidate);
    candidate.bits() > 1 && is_prime(Flavor::Any, &candidate)
}

/// A random prime of exactly `bits` bits, optionally safe or congruent to
/// `rem` modulo `add` as OpenSSL's `BN_generate_prime_ex` allows.
pub(super) fn generate_prime(
    bits: u32,
    safe: bool,
    add: Option<&[u8]>,
    rem: Option<&[u8]>,
) -> Result<Vec<u8>, &'static str> {
    let flavor = if safe { Flavor::Safe } else { Flavor::Any };
    if bits < 2 || (safe && bits < 3) {
        return Err("bits is too small");
    }
    let Some(add) = add else {
        return Ok(bytes(&random_prime(&mut rng(), flavor, bits)));
    };
    let add = NonZero::new(integer(add))
        .into_option()
        .ok_or("add must be non-zero")?;
    let rem = rem.map_or_else(BoxedUint::one, integer);
    let top = BoxedUint::one()
        .resize(bits)
        .shl_vartime(bits - 1)
        .ok_or("bits is out of range")?;
    loop {
        let random = BoxedUint::random_bits(&mut rng(), bits - 1).resize(bits);
        let candidate = random.wrapping_add(&top);
        let offset = candidate.rem_vartime(&add);
        let candidate = candidate.wrapping_sub(&offset).wrapping_add(&rem);
        if candidate.bits() == bits && is_prime(flavor, &candidate) {
            return Ok(bytes(&candidate));
        }
    }
}

/// Diffie-Hellman parameters: a safe prime for which `generator` generates
/// the prime-order subgroup, as OpenSSL chooses them.
pub(super) fn dh_parameters(bits: u32, generator: u32) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    if bits < 3 {
        return Err("bits is too small");
    }
    if generator < 2 {
        return Err("generator must be at least 2");
    }
    let (modulus, residue): (u64, u64) = match generator {
        2 => (8, 7),
        5 => (5, 4),
        _ => (1, 0),
    };
    let modulus = NonZero::new(BoxedUint::from(modulus))
        .into_option()
        .ok_or("invalid modulus")?;
    let prime = loop {
        let prime: BoxedUint = random_prime(&mut rng(), Flavor::Safe, bits);
        if prime.rem_vartime(&modulus) == BoxedUint::from(residue) {
            break prime;
        }
    };
    Ok((bytes(&prime), bytes(&BoxedUint::from(u64::from(generator)))))
}

/// A random private value in `2..prime`.
pub(super) fn random_below(prime: &[u8]) -> Result<Vec<u8>, &'static str> {
    let prime = NonZero::new(integer(prime))
        .into_option()
        .ok_or("prime must be non-zero")?;
    loop {
        let value = BoxedUint::random_mod_vartime(&mut rng(), &prime);
        if value.bits() > 1 {
            return Ok(bytes(&value));
        }
    }
}

/// `base ^ exponent mod modulus` for an odd modulus.
pub(super) fn mod_pow(
    base: &[u8],
    exponent: &[u8],
    modulus: &[u8],
) -> Result<Vec<u8>, &'static str> {
    let modulus = Odd::new(integer(modulus))
        .into_option()
        .ok_or("modulus must be odd")?;
    let params = BoxedMontyParams::new(modulus.clone());
    let base = integer(base)
        .rem_vartime(modulus.as_nz_ref())
        .resize(params.bits_precision());
    let result = BoxedMontyForm::new(base, &params).pow(&integer(exponent));
    Ok(bytes(&result.retrieve()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16))
            .collect::<Result<_, _>>()
            .unwrap_or_default()
    }

    #[test]
    fn mod_pow_matches_small_and_large_references() -> Result<(), &'static str> {
        assert_eq!(mod_pow(&[2], &[10], &[233])?, [92]);
        // RFC 2409 group 2 prime; expected value computed with Python's pow().
        let prime = hex(
            "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF",
        );
        let exponent = hex("0123456789abcdef0123456789abcdef");
        let public = mod_pow(&[2], &exponent, &prime)?;
        assert_eq!(&public[..16], &hex("3140c5c3f2e15e04e63706a0b5580300")[..]);
        Ok(())
    }

    #[test]
    fn primes_are_recognised() {
        assert!(check_prime(&[97]));
        assert!(!check_prime(&[2, 49]));
        assert!(!check_prime(&[1]));
        assert!(check_prime(&hex("1fffffffffffffff")));
    }
}
