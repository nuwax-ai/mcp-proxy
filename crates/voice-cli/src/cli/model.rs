use crate::VoiceCliError;
use crate::models::Config;
use crate::services::ModelService;
use tracing::{error, info, warn};

pub async fn handle_model_download(config: &Config, model_name: &str) -> crate::Result<()> {
    info!("Downloading model: {}", model_name);

    // Validate model name
    if !config
        .whisper
        .supported_models
        .contains(&model_name.to_string())
    {
        return Err(VoiceCliError::InvalidModelName(format!(
            "Model '{}' is not supported. Supported models: {:?}",
            model_name, config.whisper.supported_models
        )));
    }

    let model_service = ModelService::new(config.clone());

    // Check if model already exists
    if model_service.is_model_downloaded(model_name).await? {
        info!("Model '{}' already exists locally", model_name);
        return Ok(());
    }

    // Download the model
    match model_service.download_model(model_name).await {
        Ok(_) => {
            info!("Successfully downloaded model: {}", model_name);
        }
        Err(e) => {
            error!("Failed to download model '{}': {}", model_name, e);
            return Err(e);
        }
    }

    Ok(())
}

pub async fn handle_model_list(config: &Config) -> crate::Result<()> {
    info!("Listing models...");

    let model_service = ModelService::new(config.clone());

    println!("\n=== Available Models ===");
    for model in &config.whisper.supported_models {
        let is_downloaded = model_service
            .is_model_downloaded(model)
            .await
            .unwrap_or(false);
        let status = if is_downloaded {
            "✓ Downloaded"
        } else {
            "○ Not Downloaded"
        };
        let default_marker = if model == &config.whisper.default_model {
            " (default)"
        } else {
            ""
        };
        println!("  {} {}{}", status, model, default_marker);
    }

    println!("\n=== Downloaded Models Info ===");
    let downloaded_models = model_service.list_downloaded_models().await?;

    if downloaded_models.is_empty() {
        println!("  No models downloaded yet");
    } else {
        for model_name in downloaded_models {
            match model_service.get_model_info(&model_name).await {
                Ok(info) => {
                    println!(
                        "  {} - Size: {}, Status: {}",
                        model_name, info.size, info.status
                    );
                }
                Err(e) => {
                    println!("  {} - Error: {}", model_name, e);
                }
            }
        }
    }

    println!("\nModels directory: {}", config.whisper.models_dir);
    println!("Default model: {}", config.whisper.default_model);

    Ok(())
}

pub async fn handle_model_validate(config: &Config) -> crate::Result<()> {
    info!("Validating all downloaded models...");

    let model_service = ModelService::new(config.clone());
    let downloaded_models = model_service.list_downloaded_models().await?;

    if downloaded_models.is_empty() {
        info!("No models to validate");
        return Ok(());
    }

    let mut all_valid = true;

    for model_name in downloaded_models {
        print!("Validating {}... ", model_name);
        match model_service.validate_model(&model_name).await {
            Ok(_) => {
                println!("✓ Valid");
            }
            Err(e) => {
                println!("✗ Invalid: {}", e);
                all_valid = false;
            }
        }
    }

    if all_valid {
        info!("All models are valid");
    } else {
        warn!("Some models have validation issues");
    }

    Ok(())
}

pub async fn handle_model_remove(config: &Config, model_name: &str) -> crate::Result<()> {
    info!("Removing model: {}", model_name);

    let model_service = ModelService::new(config.clone());

    // Check if model exists
    if !model_service.is_model_downloaded(model_name).await? {
        warn!("Model '{}' is not downloaded", model_name);
        return Ok(());
    }

    // Confirm deletion (in a real CLI, you might want user confirmation)
    if model_name == config.whisper.default_model {
        warn!("Warning: Removing the default model '{}'", model_name);
    }

    match model_service.remove_model(model_name).await {
        Ok(_) => {
            info!("Successfully removed model: {}", model_name);
        }
        Err(e) => {
            error!("Failed to remove model '{}': {}", model_name, e);
            return Err(e);
        }
    }

    Ok(())
}

/// Interactive model download - downloads the default model if none exist
pub async fn ensure_default_model(config: &Config) -> crate::Result<()> {
    let model_service = ModelService::new(config.clone());

    // Check if default model exists
    if model_service
        .is_model_downloaded(&config.whisper.default_model)
        .await?
    {
        return Ok(());
    }

    // Check if any model exists
    let downloaded_models = model_service.list_downloaded_models().await?;
    if !downloaded_models.is_empty() {
        return Ok(());
    }

    // No models exist, download default
    if config.whisper.auto_download {
        info!(
            "No models found. Auto-downloading default model: {}",
            config.whisper.default_model
        );
        handle_model_download(config, &config.whisper.default_model).await?;
    } else {
        return Err(VoiceCliError::ModelNotFound(format!(
            "No models found and auto_download is disabled. Please run: voice-cli model download {}",
            config.whisper.default_model
        )));
    }

    Ok(())
}

/// Diagnose issues with a downloaded model
pub async fn handle_model_diagnose(config: &Config, model_name: &str) -> crate::Result<()> {
    info!("Diagnosing model: {}", model_name);

    let model_service = ModelService::new(config.clone());

    match model_service.diagnose_model(model_name).await {
        Ok(diagnosis) => {
            println!("\n=== Model Diagnosis for '{}' ===", model_name);
            println!("{}", diagnosis);

            // Provide fix suggestions
            println!("\n=== Fix Suggestions ===");
            if !model_service
                .is_model_downloaded(model_name)
                .await
                .unwrap_or(false)
            {
                println!(
                    "💡 Model not found - run: voice-cli model download {}",
                    model_name
                );
            } else {
                match model_service.validate_model(model_name).await {
                    Ok(_) => {
                        println!("✓ Model is valid and ready to use");
                    }
                    Err(_) => {
                        println!("🔧 To fix the corrupted model:");
                        println!(
                            "   1. Remove the corrupted file: voice-cli model remove {}",
                            model_name
                        );
                        println!(
                            "   2. Re-download the model: voice-cli model download {}",
                            model_name
                        );
                        println!("   3. Validate the model: voice-cli model validate");
                    }
                }
            }
        }
        Err(e) => {
            error!("Failed to diagnose model '{}': {}", model_name, e);
            return Err(e);
        }
    }

    Ok(())
}
