use std::error::Error;
use vergen_gitcl::{Emitter, GitclBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    // Emit the instructions
    Emitter::new()
        .add_instructions(&GitclBuilder::default().sha(true).build()?)?
        .emit()?;

    Ok(())
}
