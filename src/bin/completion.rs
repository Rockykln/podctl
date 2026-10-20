use podctl::completions;

pub fn emit(shell: Option<&str>) -> anyhow::Result<()> {
    match shell {
        Some(sh) => match completions::script(sh) {
            Some(text) => {
                print!("{text}");
                Ok(())
            }
            None => anyhow::bail!("unknown shell '{sh}' — try bash, zsh or fish"),
        },
        None => anyhow::bail!("usage: podctl completion <bash|zsh|fish>"),
    }
}
