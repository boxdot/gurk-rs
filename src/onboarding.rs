use dialoguer::{Input, Password, Select, theme::ColorfulTheme};

use crate::{
    config::{Config, User},
    passphrase::Passphrase,
};

pub fn run() -> anyhow::Result<(Config, Passphrase)> {
    let theme = ColorfulTheme::default();

    println!("Welcome to {}!", theme.prompt_style.apply_to("gurk"));

    println!();
    println!("Please enter your display name:");
    println!(
        "{}",
        theme.hint_style.apply_to(
            "The display name is only used within the app to identify the user locally. \
            It is not shared with other users or transmitted over the network."
        )
    );

    let display_name: String = loop {
        let display_name: String = Input::with_theme(&theme)
            .with_prompt("Display name")
            .interact_text()?;
        if !display_name.is_empty() {
            break display_name;
        }
    };

    println!();
    println!("Please enter a passphrase:");
    println!(
        "{}",
        theme.hint_style.apply_to(
            "The passphrase will be used to encrypt local (sqlite) databases \
            containing your messages and chats, Signal sessions and encryption keys."
        )
    );

    let passphrase: Passphrase = loop {
        let passphrase: String = Password::with_theme(&theme)
            .with_prompt("Passphrase")
            .with_confirmation("Confirm", "Passphrase mismatching")
            .allow_empty_password(false)
            .interact()?;
        match Passphrase::new(passphrase.clone()) {
            Ok(value) => break value,
            Err(e) => {
                println!("Invalid passphrase: {e}");
            }
        }
    };

    println!();
    let passphrase_storage = Select::with_theme(&theme)
        .with_prompt("Where do you want to store your passphrase?")
        .items(&[
            "Config file",
            #[cfg(target_os = "macos")]
            "Keychain (macOS)",
            "Don't store it (prompt on startup or CLI argument)",
        ])
        .interact()?;
    let passphrase_storage = PassphraseStorage::from(passphrase_storage);

    let mut config = Config::with_user(User { display_name });
    match passphrase_storage {
        PassphraseStorage::ConfigFile => {
            config.passphrase = Some(passphrase.clone());
        }
        #[cfg(target_os = "macos")]
        PassphraseStorage::Keychain => {
            passphrase.store_in_keychain(&config.user.display_name)?;
        }
        PassphraseStorage::DontStore => {}
    }

    let config_path = config.save_new()?;

    println!();
    println!(
        "Configuration is saved to: {}",
        theme.values_style.apply_to(config_path.display())
    );
    println!(
        "Messages and attachments will be saved to: {}",
        theme.values_style.apply_to(config.data_dir.display())
    );

    Ok((config, passphrase))
}

enum PassphraseStorage {
    #[cfg(target_os = "macos")]
    Keychain,
    ConfigFile,
    DontStore,
}

impl From<usize> for PassphraseStorage {
    fn from(value: usize) -> Self {
        #[cfg(target_os = "macos")]
        {
            match value {
                0 => PassphraseStorage::Keychain,
                1 => PassphraseStorage::ConfigFile,
                2 => PassphraseStorage::DontStore,
                _ => unreachable!("logic error"),
            }
        }
        #[cfg(not(target_os = "macos"))]
        match value {
            0 => PassphraseStorage::ConfigFile,
            1 => PassphraseStorage::DontStore,
            _ => unreachable!("logic error"),
        }
    }
}
