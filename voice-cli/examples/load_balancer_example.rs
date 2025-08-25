use std::sync::Arc;
use tempfile::tempdir;
use voice_cli::{
    load_balancer::VoiceCliLoadBalancer,
    models::{LoadBalancerConfig, MetadataStore},
};

/// Example demonstrating how to use the VoiceCliLoadBalancer
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🚀 VoiceCliLoadBalancer Example");
    println!("================================");

    // Create a temporary directory for the metadata store
    let temp_dir = tempdir()?;
    let db_path = temp_dir.path().join("metadata.db");

    // Initialize metadata store
    let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap())?);
    println!("✅ Metadata store initialized at: {:?}", db_path);

    // Configure the load balancer
    let config = LoadBalancerConfig {
        enabled: true,
        bind_address: "127.0.0.1".to_string(),
        port: 8090,
        health_check_interval: 5,
        health_check_timeout: 3,
        pid_file: "./voice-cli-lb.pid".to_string(),
        log_file: "./logs/lb.log".to_string(),
    };

    println!("⚙️  Load balancer configuration:");
    println!("   - Port: {}", config.port);
    println!(
        "   - Health check interval: {}s",
        config.health_check_interval
    );
    println!(
        "   - Health check timeout: {}s",
        config.health_check_timeout
    );

    // Create the load balancer
    let mut load_balancer = VoiceCliLoadBalancer::new(config, metadata_store).await?;
    println!("✅ VoiceCliLoadBalancer created successfully");

    // Get initial status
    let status = load_balancer.get_status().await;
    println!("📊 Initial status:");
    println!("   - Instance ID: {}", status.instance_id);
    println!("   - Total nodes: {}", status.cluster_status.total_nodes);
    println!(
        "   - Healthy nodes: {}",
        status.cluster_status.healthy_nodes
    );
    println!(
        "   - Total requests: {}",
        status.routing_stats.total_requests
    );

    // Get routing statistics
    let routing_stats = load_balancer.get_routing_stats().await;
    println!("📈 Routing statistics:");
    println!("   - Total requests: {}", routing_stats.total_requests);
    println!(
        "   - Successful requests: {}",
        routing_stats.successful_requests
    );
    println!("   - Failed requests: {}", routing_stats.failed_requests);
    println!(
        "   - Circuit breaker activations: {}",
        routing_stats.circuit_breaker_activations
    );

    // Get circuit breaker status
    let circuit_breakers = load_balancer.get_circuit_breaker_status().await;
    println!("🔌 Circuit breakers: {} active", circuit_breakers.len());

    println!("🎯 Load balancer is ready to start!");
    println!("   To start the load balancer, call: load_balancer.start().await");
    println!("   This will start all background services:");
    println!("   - Health checker");
    println!("   - Service manager");
    println!("   - HTTP proxy service");
    println!("   - Event processors");

    // Note: In a real application, you would call:
    // load_balancer.start().await?;
    // But for this example, we'll just show the setup

    println!("✨ Example completed successfully!");

    Ok(())
}
