use ossa_crdt::register::lww::LWW;
use ossa_crdt::time::{CausalState, CausalTime, ConcretizeTime};
use ossa_crdt::CRDT;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TestTime(u64);

/// A simple causal state that treats the natural ordering of TestTime as causal ordering.
struct SimpleCausalState;

impl CausalState for SimpleCausalState {
    type Time = TestTime;

    fn happens_before(&self, t1: &TestTime, t2: &TestTime) -> bool {
        t1.0 < t2.0
    }
}

impl ConcretizeTime<u64> for TestTime {
    type Serialized = CausalTime<TestTime>;

    fn concretize_time(src: Self::Serialized, current_header: u64) -> Self {
        match src {
            CausalTime::Current {
                operation_position,
            } => TestTime(current_header + operation_position as u64),
            CausalTime::Time(t) => t,
        }
    }
}

#[derive(Clone, Debug, CRDT)]
#[crdt(bound = "Time: Ord", concretize_time, concretize_time_op)]
pub struct Recipe<Time> {
    pub title: LWW<Time, String>,
    pub ingredients: LWW<Time, Vec<String>>,
    pub instructions: LWW<Time, String>,
}

#[test]
fn test_derive_generates_op_enum_and_apply() {
    let recipe = Recipe {
        title: LWW::new(TestTime(1), "Pancakes".to_string()),
        ingredients: LWW::new(TestTime(1), vec!["flour".to_string(), "eggs".to_string()]),
        instructions: LWW::new(TestTime(1), "Mix and cook.".to_string()),
    };

    let st = SimpleCausalState;

    // Apply a title update at a later time.
    let new_title = LWW::new(TestTime(2), "Fluffy Pancakes".to_string());
    let recipe = recipe.apply(&st, RecipeOp::Title(new_title));
    assert_eq!(recipe.title.value(), "Fluffy Pancakes");
    assert_eq!(recipe.title.time(), &TestTime(2));

    // Other fields unchanged.
    assert_eq!(recipe.ingredients.value().len(), 2);
    assert_eq!(recipe.instructions.value(), "Mix and cook.");

    // Apply an ingredients update.
    let new_ingredients = LWW::new(TestTime(3), vec![
        "flour".to_string(),
        "eggs".to_string(),
        "milk".to_string(),
    ]);
    let recipe = recipe.apply(&st, RecipeOp::Ingredients(new_ingredients));
    assert_eq!(recipe.ingredients.value().len(), 3);

    // Apply an instructions update.
    let new_instructions = LWW::new(TestTime(4), "Mix well and cook on low heat.".to_string());
    let recipe = recipe.apply(&st, RecipeOp::Instructions(new_instructions));
    assert_eq!(recipe.instructions.value(), "Mix well and cook on low heat.");
}

#[test]
fn test_earlier_timestamp_does_not_overwrite() {
    let recipe = Recipe {
        title: LWW::new(TestTime(5), "Waffles".to_string()),
        ingredients: LWW::new(TestTime(5), vec!["batter".to_string()]),
        instructions: LWW::new(TestTime(5), "Pour and press.".to_string()),
    };

    let st = SimpleCausalState;

    // Apply a title update with an earlier time -- should not overwrite.
    let old_title = LWW::new(TestTime(3), "Old Title".to_string());
    let recipe = recipe.apply(&st, RecipeOp::Title(old_title));
    assert_eq!(recipe.title.value(), "Waffles");
    assert_eq!(recipe.title.time(), &TestTime(5));
}

#[test]
fn test_concretize_time_current() {
    // Create a serialized op with a CausalTime::Current reference.
    let serialized_op = RecipeOp::<CausalTime<TestTime>>::Title(LWW::new(
        CausalTime::Current {
            operation_position: 0,
        },
        "Pancakes".to_string(),
    ));

    // Concretize with header id 42.
    let concrete_op: RecipeOp<TestTime> = ConcretizeTime::concretize_time(serialized_op, 42u64);

    match concrete_op {
        RecipeOp::Title(lww) => {
            assert_eq!(lww.time(), &TestTime(42));
            assert_eq!(lww.value(), "Pancakes");
        }
        _ => panic!("expected Title variant"),
    }
}

#[test]
fn test_concretize_time_absolute() {
    // Create a serialized op with an absolute CausalTime::Time reference.
    let serialized_op = RecipeOp::<CausalTime<TestTime>>::Ingredients(LWW::new(
        CausalTime::Time(TestTime(99)),
        vec!["sugar".to_string()],
    ));

    let concrete_op: RecipeOp<TestTime> = ConcretizeTime::concretize_time(serialized_op, 42u64);

    match concrete_op {
        RecipeOp::Ingredients(lww) => {
            assert_eq!(lww.time(), &TestTime(99));
            assert_eq!(lww.value(), &vec!["sugar".to_string()]);
        }
        _ => panic!("expected Ingredients variant"),
    }
}

#[test]
fn test_struct_concretize_time() {
    // Create a serialized Recipe with CausalTime fields.
    let serialized_recipe = Recipe {
        title: LWW::new(
            CausalTime::Current {
                operation_position: 0,
            },
            "Pancakes".to_string(),
        ),
        ingredients: LWW::new(
            CausalTime::Time(TestTime(10)),
            vec!["flour".to_string()],
        ),
        instructions: LWW::new(
            CausalTime::Current {
                operation_position: 1,
            },
            "Mix.".to_string(),
        ),
    };

    let concrete: Recipe<TestTime> =
        ConcretizeTime::concretize_time(serialized_recipe, 42u64);

    // Current { operation_position: 0 } -> TestTime(42 + 0)
    assert_eq!(concrete.title.time(), &TestTime(42));
    assert_eq!(concrete.title.value(), "Pancakes");

    // Time(TestTime(10)) -> TestTime(10)
    assert_eq!(concrete.ingredients.time(), &TestTime(10));
    assert_eq!(concrete.ingredients.value(), &vec!["flour".to_string()]);

    // Current { operation_position: 1 } -> TestTime(42 + 1)
    assert_eq!(concrete.instructions.time(), &TestTime(43));
    assert_eq!(concrete.instructions.value(), "Mix.");
}
