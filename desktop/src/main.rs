use massive_shell::ApplicationContext;
use massive_shell::Result;
use massive_shell::shell;

fn main() -> Result<()> {
    shell::run(application)
}

async fn application(_context: ApplicationContext) -> Result<()> {
    println!("Application");
    Ok(())
}
