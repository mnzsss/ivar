#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn normalize_checkboxes_handles_dash_star_table_and_indentation() {
    let input = "\
# Plan

- [ ] Unchecked task
- [x] Checked task lower
- [X] Checked task upper
  - [x] Indented checked task
    * [x] Deeply indented star checked
* [ ] Star unchecked
* [x] Star checked lower
* [X] Star checked upper
| [x] | Table checked lower |
| [X] | Table checked upper |
| [ ] | Table unchecked |
Regular paragraph text with [x] in middle
";

    let expected = "\
# Plan

- [ ] Unchecked task
- [ ] Checked task lower
- [ ] Checked task upper
  - [ ] Indented checked task
    * [ ] Deeply indented star checked
* [ ] Star unchecked
* [ ] Star checked lower
* [ ] Star checked upper
| [ ] | Table checked lower |
| [ ] | Table checked upper |
| [ ] | Table unchecked |
Regular paragraph text with [x] in middle
";

    assert_eq!(normalize_checkboxes(input), expected);
}

#[test]
fn normalize_checkboxes_preserves_trailing_newline_or_lack_thereof() {
    let with_nl = "- [x] task\n";
    assert_eq!(normalize_checkboxes(with_nl), "- [ ] task\n");

    let without_nl = "- [x] task";
    assert_eq!(normalize_checkboxes(without_nl), "- [ ] task");

    assert_eq!(normalize_checkboxes(""), "");
}
