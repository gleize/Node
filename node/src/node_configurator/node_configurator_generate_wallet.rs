// Copyright (c) 2017-2019, Substratum LLC (https://substratum.net) and/or its affiliates. All rights reserved.

use crate::blockchain::bip32::Bip32ECKeyPair;
use crate::blockchain::bip39::Bip39;
use crate::multi_config::MultiConfig;
use crate::node_configurator::node_configurator::{
    common_validators, config_file_arg, consuming_wallet_arg, create_wallet, data_directory_arg,
    earning_wallet_arg, flushed_write, initialize_database, language_arg, make_multi_config,
    mnemonic_passphrase_arg, request_new_password, wallet_password_arg, Either, NodeConfigurator,
    PasswordError, WalletCreationConfig, WalletCreationConfigMaker, EARNING_WALLET_HELP,
    WALLET_PASSWORD_HELP,
};
use crate::persistent_configuration::PersistentConfiguration;
use crate::sub_lib::cryptde::PlainData;
use crate::sub_lib::main_tools::StdStreams;
use crate::sub_lib::wallet::Wallet;
use bip39::{Language, Mnemonic, MnemonicType};
use clap::{crate_authors, crate_description, crate_version, value_t, App, AppSettings, Arg};
use indoc::indoc;
use std::str::FromStr;

pub struct NodeConfiguratorGenerateWallet {
    app: App<'static, 'static>,
    mnemonic_factory: Box<MnemonicFactory>,
}

impl NodeConfigurator<WalletCreationConfig> for NodeConfiguratorGenerateWallet {
    fn configure(&self, args: &Vec<String>, streams: &mut StdStreams<'_>) -> WalletCreationConfig {
        let multi_config = make_multi_config(&self.app, args);
        let persistent_config = initialize_database(&multi_config);

        let config = self.parse_args(&multi_config, streams, persistent_config.as_ref());

        create_wallet(&config, persistent_config.as_ref());

        config
    }
}

pub trait MnemonicFactory {
    fn make(&self, mnemonic_type: MnemonicType, language: Language) -> Mnemonic;
}

struct MnemonicFactoryReal {}

impl MnemonicFactory for MnemonicFactoryReal {
    fn make(&self, mnemonic_type: MnemonicType, language: Language) -> Mnemonic {
        Bip39::mnemonic(mnemonic_type, language)
    }
}

const GENERATE_WALLET_HELP: &str =
    "Generate a new set of HD wallets with mnemonic recovery phrase from the standard \
     BIP39 predefined list of words. Not valid as a configuration file item nor an \
     environment variable";
const WORD_COUNT_HELP: &str =
    "The number of words in the mnemonic phrase. Ropsten defaults to 12 words. \
     Mainnet defaults to 24 words.";

const HELP_TEXT: &str = indoc!(
    r"ADDITIONAL HELP:
    If you want to generate wallets to earn money into and spend money from, try:

        SubstratumNode --help --generate-wallet

    If the Node is already configured with your wallets, and you want to start the Node so that it
    stays running:

        SubstratumNode --help"
);

impl WalletCreationConfigMaker for NodeConfiguratorGenerateWallet {
    fn make_mnemonic_passphrase(
        &self,
        multi_config: &MultiConfig,
        streams: &mut StdStreams,
    ) -> String {
        match value_m!(multi_config, "mnemonic-passphrase", String) {
            Some(mp) => mp,
            None => match Self::request_mnemonic_passphrase(streams) {
                Some(mp) => mp,
                None => "".to_string(),
            },
        }
    }

    fn make_mnemonic_seed(
        &self,
        multi_config: &MultiConfig,
        streams: &mut StdStreams,
        mnemonic_passphrase: &str,
        consuming_derivation_path: &str,
        earning_wallet_info: &Either<String, String>,
    ) -> PlainData {
        let language_str =
            value_m!(multi_config, "language", String).expect("--language is not defaulted");
        let language = Bip39::language_from_name(&language_str);
        let word_count =
            value_m!(multi_config, "word-count", usize).expect("--word-count is not defaulted");
        let mnemonic_type = MnemonicType::for_word_count(word_count)
            .expect("--word-count is not properly value-restricted");
        let mnemonic = self.mnemonic_factory.make(mnemonic_type, language);
        let seed = PlainData::new(Bip39::seed(&mnemonic, &mnemonic_passphrase).as_ref());
        Self::report_wallet_information(
            streams,
            &mnemonic,
            &seed,
            &consuming_derivation_path,
            &earning_wallet_info,
        );
        seed
    }
}

impl NodeConfiguratorGenerateWallet {
    pub fn new() -> Self {
        Self {
            app: App::new("SubstratumNode")
                .global_settings(if cfg!(test) {
                    &[AppSettings::ColorNever]
                } else {
                    &[AppSettings::ColorAuto, AppSettings::ColoredHelp]
                })
                .version(crate_version!())
                .author(crate_authors!("\n"))
                .about(crate_description!())
                .after_help(HELP_TEXT)
                .arg(
                    Arg::with_name("generate-wallet")
                        .long("generate-wallet")
                        .aliases(&["generate-wallet", "generate_wallet"])
                        .required(true)
                        .takes_value(false)
                        .requires_all(&["language", "word-count"])
                        .help(GENERATE_WALLET_HELP),
                )
                .arg(config_file_arg())
                .arg(consuming_wallet_arg())
                .arg(data_directory_arg())
                .arg(earning_wallet_arg(
                    EARNING_WALLET_HELP,
                    common_validators::validate_earning_wallet,
                ))
                .arg(language_arg())
                .arg(mnemonic_passphrase_arg())
                .arg(wallet_password_arg(WALLET_PASSWORD_HELP))
                .arg(
                    Arg::with_name("word-count")
                        .long("word-count")
                        .aliases(&["word-count", "word_count"])
                        .required(true)
                        .value_name("WORD-COUNT")
                        .possible_values(&["12", "15", "18", "21", "24"])
                        .default_value("12")
                        .help(WORD_COUNT_HELP),
                ),
            mnemonic_factory: Box::new(MnemonicFactoryReal {}),
        }
    }

    fn parse_args(
        &self,
        multi_config: &MultiConfig,
        streams: &mut StdStreams<'_>,
        persistent_config: &PersistentConfiguration,
    ) -> WalletCreationConfig {
        match persistent_config.encrypted_mnemonic_seed() {
            Some(_) => panic!("Can't generate wallets: mnemonic seed has already been created"),
            None => (),
        }
        self.make_wallet_creation_config(multi_config, streams)
    }

    fn request_mnemonic_passphrase(streams: &mut StdStreams) -> Option<String> {
        flushed_write (streams.stdout, "\nPlease provide an extra mnemonic passphrase to ensure your wallet is unique (NOTE: \
            This passphrase cannot be changed later and still produce the same addresses). You will \
            encrypt your wallet in a following step...\n",
        );
        flushed_write(streams.stdout, "Mnemonic passphrase (recommended): ");
        match request_new_password(
            "Confirm mnemonic passphrase: ",
            "Passphrases do not match.",
            streams,
            |_| Ok(()),
        ) {
            Ok(mp) => {
                if mp.is_empty() {
                    flushed_write (
                        streams.stdout,
                        "\nWhile ill-advised, proceeding with no mnemonic passphrase.\nPress Enter to continue...",
                    );
                    let _ = streams.stdin.read(&mut [0u8]).is_ok();
                    None
                } else {
                    Some(mp)
                }
            }
            Err(PasswordError::Mismatch) => panic!("Passphrases do not match."),
            Err(e) => panic!("{:?}", e),
        }
    }

    fn report_wallet_information(
        streams: &mut StdStreams<'_>,
        mnemonic: &Mnemonic,
        seed: &PlainData,
        consuming_derivation_path: &str,
        earning_wallet_info: &Either<String, String>,
    ) {
        flushed_write(
            streams.stdout,
            "\n\nRecord the following mnemonic recovery \
             phrase in the sequence provided and keep it secret! \
             You cannot recover your wallet without these words \
             plus your mnemonic passphrase if you provided one.\n\n",
        );
        flushed_write(streams.stdout, &format!("{}", mnemonic.phrase()));
        flushed_write(streams.stdout, "\n\n");
        let consuming_keypair = Bip32ECKeyPair::from_raw(seed.as_ref(), &consuming_derivation_path)
            .expect(&format!(
                "Couldn't make key pair from consuming derivation path '{}'",
                consuming_derivation_path
            ));
        let consuming_wallet = Wallet::from(consuming_keypair);
        flushed_write(
            streams.stdout,
            &format!(
                "Consuming Wallet ({}): {}\n",
                consuming_derivation_path, consuming_wallet
            ),
        );
        match &earning_wallet_info {
            Either::Left(address) => {
                let earning_wallet =
                    Wallet::from_str(address).expect("Address doesn't work anymore");
                flushed_write(
                    streams.stdout,
                    &format!("  Earning Wallet: {}", earning_wallet),
                );
            }
            Either::Right(earning_derivation_path) => {
                let earning_keypair =
                    Bip32ECKeyPair::from_raw(seed.as_ref(), &earning_derivation_path).expect(
                        &format!(
                            "Couldn't make key pair from earning derivation path '{}'",
                            earning_derivation_path
                        ),
                    );
                let earning_wallet = Wallet::from(earning_keypair.address());
                flushed_write(
                    streams.stdout,
                    &format!(
                        "  Earning Wallet ({}): {}",
                        earning_derivation_path, earning_wallet
                    ),
                );
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_dao::{ConfigDao, ConfigDaoReal};
    use crate::database::db_initializer;
    use crate::database::db_initializer::DbInitializer;
    use crate::multi_config::{CommandLineVCL, VirtualCommandLine};
    use crate::node_configurator::node_configurator::DerivationPathWalletInfo;
    use crate::persistent_configuration::PersistentConfigurationReal;
    use crate::sub_lib::cryptde::PlainData;
    use crate::sub_lib::wallet::DEFAULT_CONSUMING_DERIVATION_PATH;
    use crate::sub_lib::wallet::DEFAULT_EARNING_DERIVATION_PATH;
    use crate::test_utils::test_utils::make_default_persistent_configuration;
    use crate::test_utils::test_utils::{assert_eq_debug, ensure_node_home_directory_exists};
    use crate::test_utils::test_utils::{ByteArrayWriter, FakeStreamHolder};
    use bip39::Seed;
    use std::cell::RefCell;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    struct MnemonicFactoryMock {
        make_parameters: Arc<Mutex<Vec<(MnemonicType, Language)>>>,
        make_results: RefCell<Vec<Mnemonic>>,
    }

    impl MnemonicFactory for MnemonicFactoryMock {
        fn make(&self, mnemonic_type: MnemonicType, language: Language) -> Mnemonic {
            let mut parameters = self.make_parameters.lock().unwrap();
            parameters.push((mnemonic_type, language));
            self.make_results.borrow_mut().remove(0)
        }
    }

    impl MnemonicFactoryMock {
        pub fn new() -> MnemonicFactoryMock {
            MnemonicFactoryMock {
                make_parameters: Arc::new(Mutex::new(vec![])),
                make_results: RefCell::new(vec![]),
            }
        }

        pub fn make_parameters(
            mut self,
            parameters_arc: &Arc<Mutex<Vec<(MnemonicType, Language)>>>,
        ) -> MnemonicFactoryMock {
            self.make_parameters = parameters_arc.clone();
            self
        }

        pub fn make_result(self, result: Mnemonic) -> MnemonicFactoryMock {
            self.make_results.borrow_mut().push(result);
            self
        }
    }

    fn make_default_cli_params() -> Vec<String> {
        vec![String::from("SubstratumNode")]
    }

    #[test]
    fn parse_args_creates_configurations() {
        let home_dir = ensure_node_home_directory_exists(
            "node_configurator_generate_wallet",
            "parse_args_creates_configurations",
        );

        let password = "secret-wallet-password";
        let args: Vec<String> = vec![
            "SubstratumNode",
            "--generate-wallet",
            "--config-file",
            "specified_config.toml",
            "--data-directory",
            home_dir.to_str().unwrap(),
            "--wallet-password",
            password,
            "--consuming-wallet",
            "m/44'/60'/0'/77/78",
            "--earning-wallet",
            "m/44'/60'/0'/78/77",
            "--language",
            "español",
            "--word-count",
            "15",
            "--mnemonic-passphrase",
            "Mortimer",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let mut subject = NodeConfiguratorGenerateWallet::new();
        let make_parameters_arc = Arc::new(Mutex::new(vec![]));
        let expected_mnemonic = Mnemonic::new(MnemonicType::Words15, Language::Spanish);
        let mnemonic_factory = MnemonicFactoryMock::new()
            .make_parameters(&make_parameters_arc)
            .make_result(expected_mnemonic.clone());
        subject.mnemonic_factory = Box::new(mnemonic_factory);
        let vcls: Vec<Box<dyn VirtualCommandLine>> = vec![Box::new(CommandLineVCL::new(args))];
        let multi_config = MultiConfig::new(&subject.app, vcls);

        let config = subject.parse_args(
            &multi_config,
            &mut FakeStreamHolder::new().streams(),
            &make_default_persistent_configuration(),
        );
        let mut make_parameters = make_parameters_arc.lock().unwrap();
        assert_eq_debug(
            make_parameters.remove(0),
            (MnemonicType::Words15, Language::Spanish),
        );
        assert_eq!(
            config,
            WalletCreationConfig {
                earning_wallet_address_opt: None,
                derivation_path_info_opt: Some(DerivationPathWalletInfo {
                    mnemonic_seed: PlainData::new(
                        Seed::new(&expected_mnemonic, "Mortimer").as_ref()
                    ),
                    wallet_password: password.to_string(),
                    consuming_derivation_path_opt: Some("m/44'/60'/0'/77/78".to_string()),
                    earning_derivation_path_opt: Some("m/44'/60'/0'/78/77".to_string())
                })
            },
        );
    }

    #[test]
    fn parse_args_creates_configuration_with_defaults() {
        let args: Vec<String> = vec![
            "SubstratumNode",
            "--generate-wallet",
            "--wallet-password",
            "password123",
            "--mnemonic-passphrase",
            "Mortimer",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let mut subject = NodeConfiguratorGenerateWallet::new();
        let make_parameters_arc = Arc::new(Mutex::new(vec![]));
        let expected_mnemonic = Mnemonic::new(MnemonicType::Words12, Language::English);
        let mnemonic_factory = MnemonicFactoryMock::new()
            .make_parameters(&make_parameters_arc)
            .make_result(expected_mnemonic.clone());
        subject.mnemonic_factory = Box::new(mnemonic_factory);
        let vcls: Vec<Box<dyn VirtualCommandLine>> = vec![Box::new(CommandLineVCL::new(args))];
        let multi_config = MultiConfig::new(&subject.app, vcls);

        let config = subject.parse_args(
            &multi_config,
            &mut FakeStreamHolder::new().streams(),
            &make_default_persistent_configuration(),
        );

        let mut make_parameters = make_parameters_arc.lock().unwrap();
        assert_eq_debug(
            make_parameters.remove(0),
            (MnemonicType::Words12, Language::English),
        );
        assert_eq!(
            config,
            WalletCreationConfig {
                earning_wallet_address_opt: None,
                derivation_path_info_opt: Some(DerivationPathWalletInfo {
                    mnemonic_seed: PlainData::new(
                        Seed::new(&expected_mnemonic, "Mortimer").as_ref()
                    ),
                    wallet_password: "password123".to_string(),
                    consuming_derivation_path_opt: Some(
                        DEFAULT_CONSUMING_DERIVATION_PATH.to_string()
                    ),
                    earning_derivation_path_opt: Some(DEFAULT_EARNING_DERIVATION_PATH.to_string())
                })
            },
        );
    }

    #[test]
    #[should_panic(expected = "Passphrases do not match.")]
    fn make_mnemonic_passphrase_panics_after_three_passphrase_mismatches() {
        let subject = NodeConfiguratorGenerateWallet::new();
        let streams = &mut StdStreams {
            stdin: &mut Cursor::new(&b"one\neno\ntwo\nowt\nthree\neerht\n"[..]),
            stdout: &mut ByteArrayWriter::new(),
            stderr: &mut ByteArrayWriter::new(),
        };
        let args: Vec<String> = vec!["SubstratumNode", "--generate-wallet"]
            .into_iter()
            .map(String::from)
            .collect();
        let multi_config = MultiConfig::new(
            &subject.app,
            vec![Box::new(CommandLineVCL::new(args.clone()))],
        );
        subject.make_mnemonic_passphrase(&multi_config, streams);
    }

    #[test]
    fn make_mnemonic_passphrase_allows_blank_passphrase_with_scolding() {
        let args: Vec<String> = vec!["SubstratumNode", "--generate-wallet"]
            .into_iter()
            .map(String::from)
            .collect();

        let mut subject = NodeConfiguratorGenerateWallet::new();
        let mnemonic = Mnemonic::new(MnemonicType::Words12, Language::English);
        let mnemonic_factory = MnemonicFactoryMock::new().make_result(mnemonic.clone());
        subject.mnemonic_factory = Box::new(mnemonic_factory);
        let stdout_writer = &mut ByteArrayWriter::new();
        let mut streams = &mut StdStreams {
            stdin: &mut Cursor::new(&b"\n\n\n"[..]),
            stdout: stdout_writer,
            stderr: &mut ByteArrayWriter::new(),
        };
        let vcl = Box::new(CommandLineVCL::new(args));
        let multi_config = MultiConfig::new(&subject.app, vec![vcl]);

        subject.make_mnemonic_passphrase(&multi_config, &mut streams);

        let captured_output = stdout_writer.get_string();
        let expected_output = "\nPlease provide an extra mnemonic passphrase to ensure your wallet is unique (NOTE: This passphrase \
                cannot be changed later and still produce the same addresses). You will encrypt your wallet in a following step...\
                \nMnemonic passphrase (recommended): Confirm mnemonic passphrase: \nWhile ill-advised, proceeding with no mnemonic passphrase.\
        \nPress Enter to continue...";
        assert_eq!(&captured_output, expected_output);
    }

    #[test]
    #[should_panic(expected = "Can't generate wallets: mnemonic seed has already been created")]
    fn preexisting_mnemonic_seed_causes_collision_and_panics() {
        let data_directory = ensure_node_home_directory_exists(
            "node_configurator_generate_wallet",
            "preexisting_mnemonic_seed_causes_collision_and_panics",
        );

        let conn = db_initializer::DbInitializerReal::new()
            .initialize(&data_directory)
            .unwrap();
        let config_dao = ConfigDaoReal::new(conn);
        config_dao.set_string("seed", "booga booga").unwrap();
        let mut args = make_default_cli_params();
        args.extend(
            vec![
                "--generate-wallet",
                "--data-directory",
                data_directory.to_str().unwrap(),
                "--wallet-password",
                "rick-rolled",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>(),
        );
        let subject = NodeConfiguratorGenerateWallet::new();
        let vcl = Box::new(CommandLineVCL::new(args));
        let multi_config = MultiConfig::new(&subject.app, vec![vcl]);

        subject.parse_args(
            &multi_config,
            &mut FakeStreamHolder::new().streams(),
            &PersistentConfigurationReal::new(Box::new(config_dao)),
        );
    }

    #[test]
    #[should_panic(expected = "could not be read: ")]
    fn configure_senses_when_user_specifies_config_file() {
        let subject = NodeConfiguratorGenerateWallet::new();
        let args = vec![
            "SubstratumNode",
            "--dns-servers",
            "1.2.3.4",
            "--config-file",
            "booga.toml", // nonexistent config file: should stimulate panic because user-specified
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<String>>();
        subject.configure(&args, &mut FakeStreamHolder::new().streams());
    }
}