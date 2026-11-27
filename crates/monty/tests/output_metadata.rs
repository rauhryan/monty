//! Runtime type metadata for output-only representations.

use monty::MontyRun;
use monty_types::{
    CompileOptions,
    unstable::{self, MontyNode},
};

/// Unsupported outputs preserve type independently of their repr payload.
#[test]
fn output_type_metadata() {
    let mut runner = MontyRun::new(
        "def source():\n    yield 1\n[{}.keys(), {}.values(), {}.items(), source(), iter([1])]".to_owned(),
        "types.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    let output = runner.run_no_limits(vec![]).unwrap();
    let items = output.as_ref().items().unwrap();
    for (item, expected) in items
        .iter()
        .zip(["dict_keys", "dict_values", "dict_items", "generator", "list_iterator"])
    {
        assert_eq!(item.type_name(), expected);
        assert!(matches!(unstable::node(*item), MontyNode::TypedRepr { .. }));
    }
}
