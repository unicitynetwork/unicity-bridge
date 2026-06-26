use std::{env, path::PathBuf, process};

fn main() {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        usage();
        return;
    };

    let result = match cmd.as_str() {
        "check-vectors" => {
            let root = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("../bridge-vectors"));
            bridge_return_host::check_vectors(&root)
        }
        "emit-b1-token-vector" => {
            let fixture = bridge_return_host::fixture::build_b1_direct_bridge_fixture();
            println!(
                "{}",
                serde_json::to_string_pretty(&bridge_return_host::fixture::b1_fixture_json(
                    &fixture
                ))
                .expect("serialize fixture")
            );
            Ok(())
        }
        "emit-split-token-vector" => {
            let fixture = bridge_return_host::fixture::build_split_bridge_fixture();
            println!(
                "{}",
                serde_json::to_string_pretty(&bridge_return_host::fixture::split_fixture_json(
                    &fixture
                ))
                .expect("serialize fixture")
            );
            Ok(())
        }
        "emit-b1-wire-input" => {
            let fixture = bridge_return_host::fixture::build_b1_direct_bridge_fixture();
            println!(
                "0x{}",
                hex::encode(bridge_return_guest::wire::encode_guest_input(
                    &fixture.input
                ))
            );
            Ok(())
        }
        "emit-split-wire-input" => {
            let fixture = bridge_return_host::fixture::build_split_bridge_fixture();
            println!(
                "0x{}",
                hex::encode(bridge_return_guest::wire::encode_guest_input(
                    &fixture.input
                ))
            );
            Ok(())
        }
        "sp1-execute" => {
            let elf = args.next().map(PathBuf::from);
            let wire = args.next();
            sp1_execute(elf, wire)
        }
        "sp1-mock-groth16" => {
            let elf = args.next().map(PathBuf::from);
            let wire = args.next();
            let proof = args.next().map(PathBuf::from);
            sp1_mock_groth16(elf, wire, proof)
        }
        "sp1-proof-info" => {
            let proof = args.next().map(PathBuf::from);
            sp1_proof_info(proof)
        }
        _ => {
            usage();
            Ok(())
        }
    };

    if let Err(err) = result {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn usage() {
    eprintln!("usage: bridge-return-host check-vectors [../bridge-vectors]");
    eprintln!("       bridge-return-host emit-b1-token-vector");
    eprintln!("       bridge-return-host emit-split-token-vector");
    eprintln!("       bridge-return-host emit-b1-wire-input");
    eprintln!("       bridge-return-host emit-split-wire-input");
    eprintln!("       bridge-return-host sp1-execute <guest.elf> <wire_hex>                 # --features sp1");
    eprintln!("       bridge-return-host sp1-mock-groth16 <guest.elf> <wire_hex> <proof.bin> # --features sp1");
    eprintln!("       bridge-return-host sp1-proof-info <proof.bin>                         # --features sp1");
}

#[cfg(feature = "sp1")]
fn require_arg<T>(value: Option<T>, name: &str) -> bridge_return_host::Result<T> {
    value.ok_or_else(|| bridge_return_host::HostError::Check(format!("missing {name}")))
}

#[cfg(feature = "sp1")]
fn decode_hex_arg(value: String, name: &str) -> bridge_return_host::Result<Vec<u8>> {
    hex::decode(value.strip_prefix("0x").unwrap_or(&value))
        .map_err(|err| bridge_return_host::HostError::Check(format!("{name}: {err}")))
}

#[cfg(feature = "sp1")]
fn sp1_execute(elf: Option<PathBuf>, wire: Option<String>) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let wire = decode_hex_arg(require_arg(wire, "wire_hex")?, "wire_hex")?;
    let execution = bridge_return_host::sp1::execute_elf(&elf, wire)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "public_values": format!("0x{}", hex::encode(&execution.public_values)),
            "expected_public_values": format!("0x{}", hex::encode(&execution.expected_public_values)),
            "cycles": execution.cycles,
        }))
        .expect("serialize SP1 execution")
    );
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_execute(_elf: Option<PathBuf>, _wire: Option<String>) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_mock_groth16(
    elf: Option<PathBuf>,
    wire: Option<String>,
    proof: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let wire = decode_hex_arg(require_arg(wire, "wire_hex")?, "wire_hex")?;
    let proof = require_arg(proof, "proof.bin")?;
    let info = bridge_return_host::sp1::mock_groth16(&elf, wire, &proof)?;
    print_proof_info(&info);
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_mock_groth16(
    _elf: Option<PathBuf>,
    _wire: Option<String>,
    _proof: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_proof_info(proof: Option<PathBuf>) -> bridge_return_host::Result<()> {
    let proof = require_arg(proof, "proof.bin")?;
    let info = bridge_return_host::sp1::proof_info_from_file(&proof)?;
    print_proof_info(&info);
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_proof_info(_proof: Option<PathBuf>) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn print_proof_info(info: &bridge_return_host::sp1::Sp1ProofInfo) {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "proof_mode": info.proof_mode,
            "sp1_version": info.sp1_version,
            "public_values": format!("0x{}", hex::encode(&info.public_values)),
            "proof_bytes": format!("0x{}", hex::encode(&info.proof_bytes)),
            "proof_bytes_len": info.proof_bytes.len(),
        }))
        .expect("serialize SP1 proof info")
    );
}
