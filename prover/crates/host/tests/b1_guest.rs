use bridge_return_core::{public_values_abi, public_values_digest};
use bridge_return_guest::{execute, execute_public_output, execute_wire, wire};
use bridge_return_host::fixture::{
    b1_fixture_json, build_b1_direct_bridge_fixture, build_split_bridge_fixture, split_fixture_json,
};

#[test]
fn guest_executes_direct_bridge_burn_b1() {
    let fixture = build_b1_direct_bridge_fixture();

    if std::env::var_os("BRIDGE_PRINT_B1_TOKEN_VECTOR").is_some() {
        println!(
            "{}",
            serde_json::to_string_pretty(&b1_fixture_json(&fixture)).unwrap()
        );
    }

    assert_eq!(execute(&fixture.input), Ok(fixture.input.public_values));
    let output = execute_public_output(&fixture.input).unwrap();
    assert_eq!(output.public_values, fixture.input.public_values);
    assert_eq!(
        output.public_values_abi,
        public_values_abi(&fixture.input.public_values)
    );
    assert_eq!(
        output.public_values_digest,
        public_values_digest(&fixture.input.public_values)
    );
    assert_wire_executes(&fixture.input);
}

#[test]
fn guest_executes_split_bridge_burn_b1() {
    let fixture = build_split_bridge_fixture();

    if std::env::var_os("BRIDGE_PRINT_SPLIT_TOKEN_VECTOR").is_some() {
        println!(
            "{}",
            serde_json::to_string_pretty(&split_fixture_json(&fixture)).unwrap()
        );
    }

    assert_eq!(execute(&fixture.input), Ok(fixture.input.public_values));
    let output = execute_public_output(&fixture.input).unwrap();
    assert_eq!(output.public_values, fixture.input.public_values);
    assert_eq!(
        output.public_values_abi,
        public_values_abi(&fixture.input.public_values)
    );
    assert_eq!(
        output.public_values_digest,
        public_values_digest(&fixture.input.public_values)
    );
    assert_wire_executes(&fixture.input);
}

fn assert_wire_executes(input: &bridge_return_guest::GuestInput) {
    let bytes = wire::encode_guest_input(input);
    assert_eq!(wire::decode_guest_input(&bytes), Ok(input.clone()));
    let output = execute_wire(&bytes).unwrap();
    assert_eq!(output.public_values, input.public_values);
    assert_eq!(
        output.public_values_abi,
        public_values_abi(&input.public_values)
    );
    assert_eq!(
        output.public_values_digest,
        public_values_digest(&input.public_values)
    );
}
