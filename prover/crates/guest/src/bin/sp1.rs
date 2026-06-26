#![no_main]

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let output = bridge_return_guest::execute_wire(&input).expect("bridge return relation failed");
    sp1_zkvm::io::commit_slice(&output.public_values_abi);
    sp1_zkvm::io::commit_slice(&output.public_values_digest);
}
