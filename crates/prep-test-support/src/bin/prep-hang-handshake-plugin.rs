use prep_test_support::{AdversarialPluginMode, run_adversarial_plugin};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_adversarial_plugin(AdversarialPluginMode::HangHandshake)
}
