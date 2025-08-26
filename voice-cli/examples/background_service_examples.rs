//! Background Service Examples
//! 
//! This file demonstrates how to use the new unified background service abstraction
//! in voice-cli. It shows various patterns and use cases for different service types.

use voice_cli::daemon::{
    DefaultServiceManager, HttpServerService, ClusterNodeService, LoadBalancerService,
    HttpServerServiceBuilder, ClusterNodeServiceBuilder, LoadBalancerServiceBuilder,
    ServiceHealth, ServiceStatus, ServiceError, BackgroundService
};
use voice_cli::models::Config;
use tracing::{info, warn, error};
use tokio::time::{sleep, Duration};

/// Example 1: Basic HTTP Server Service Usage
/// 
/// This example shows the most common use case - running an HTTP server
/// with the new background service abstraction.
async fn example_http_server_basic() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 1: Basic HTTP Server Service ===");

    // Load configuration (in practice, this would come from config files)
    let config = Config::default();

    // Create HTTP server service in background mode
    let service = HttpServerServiceBuilder::new()
        .foreground_mode(false) // Background mode for start/stop/restart
        .build();

    // Create service manager
    let mut manager = DefaultServiceManager::new(service, config, false);

    // Start the service
    println!("Starting HTTP server...");
    match manager.start().await {
        Ok(_) => {
            println!("✅ HTTP server started successfully");
            
            // Check status
            match manager.status() {
                ServiceStatus::Running => println!("📊 Status: Running"),
                status => println!("📊 Status: {:?}", status),
            }
            
            // Check health
            match manager.health().await {
                ServiceHealth::Healthy => println!("💚 Health: Healthy"),
                health => println!("💚 Health: {:?}", health),
            }
            
            // Let it run for a bit
            sleep(Duration::from_secs(2)).await;
            
            // Stop the service
            println!("Stopping HTTP server...");
            manager.stop().await?;
            println!("✅ HTTP server stopped successfully");
        }
        Err(e) => {
            println!("❌ Failed to start HTTP server: {}", e);
        }
    }

    Ok(())
}

/// Example 2: Cluster Node Service with Auto-Generated Node ID
/// 
/// This example demonstrates cluster node management with automatic node ID generation.
#[allow(dead_code)]
async fn example_cluster_node_auto_id() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 2: Cluster Node Service (Auto ID) ===");

    // Load configuration for cluster
    let mut config = Config::default();
    config.cluster.enabled = true;
    config.cluster.grpc_port = 50051;
    config.cluster.http_port = 8080;

    // Create cluster node service with auto-generated node ID
    let service = ClusterNodeServiceBuilder::new()
        .auto_node_id() // Generates a UUID-based node ID
        .foreground_mode(false)
        .build()?;

    println!("Node ID: {}", service.node_id());

    // Create service manager
    let mut manager = DefaultServiceManager::new(service, config, false);

    // Start the service
    println!("Starting cluster node...");
    match manager.start().await {
        Ok(_) => {
            println!("✅ Cluster node started successfully");
            
            // Monitor status for a while
            for i in 1..=3 {
                sleep(Duration::from_secs(1)).await;
                
                let status = manager.status();
                let health = manager.health().await;
                
                println!("Check #{}: Status={:?}, Health={:?}", i, status, health);
            }
            
            // Stop the service
            println!("Stopping cluster node...");
            manager.stop().await?;
            println!("✅ Cluster node stopped successfully");
        }
        Err(e) => {
            println!("❌ Failed to start cluster node: {}", e);
        }
    }

    Ok(())
}

/// Example 3: Load Balancer Service with Health Monitoring
/// 
/// This example shows how to run a load balancer with health monitoring.
#[allow(dead_code)]
async fn example_load_balancer_with_monitoring() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 3: Load Balancer Service with Health Monitoring ===");

    // Configure load balancer
    let mut config = Config::default();
    config.load_balancer.enabled = true;
    config.load_balancer.port = 8090;
    config.load_balancer.health_check_interval = 5;
    config.load_balancer.health_check_timeout = 3;

    // Create load balancer service
    let service = LoadBalancerServiceBuilder::new()
        .foreground_mode(false)
        .build();

    // Create service manager
    let mut manager = DefaultServiceManager::new(service, config, false);

    // Start the service
    println!("Starting load balancer...");
    match manager.start().await {
        Ok(_) => {
            println!("✅ Load balancer started successfully");
            
            // Monitor health over time
            for i in 1..=5 {
                sleep(Duration::from_secs(2)).await;
                
                let health = manager.health().await;
                let uptime = manager.uptime().map(|d| format!("{:?}", d)).unwrap_or("Unknown".to_string());
                
                println!("Health check #{}: {:?} (uptime: {})", i, health, uptime);
            }
            
            // Stop the service
            println!("Stopping load balancer...");
            manager.stop().await?;
            println!("✅ Load balancer stopped successfully");
        }
        Err(e) => {
            println!("❌ Failed to start load balancer: {}", e);
        }
    }

    Ok(())
}

/// Example 4: Service Restart Pattern
/// 
/// This example demonstrates how to handle service restarts gracefully.
#[allow(dead_code)]
async fn example_service_restart_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 4: Service Restart Pattern ===");

    let config = Config::default();
    let service = HttpServerServiceBuilder::new()
        .foreground_mode(false)
        .build();
    let mut manager = DefaultServiceManager::new(service, config, false);

    // Initial start
    println!("🚀 Starting service...");
    manager.start().await?;
    println!("✅ Service started");

    // Let it run
    sleep(Duration::from_secs(1)).await;

    // Restart the service
    println!("🔄 Restarting service...");
    manager.restart().await?;
    println!("✅ Service restarted");

    // Verify it's running
    match manager.health().await {
        ServiceHealth::Healthy => println!("💚 Service is healthy after restart"),
        health => println!("⚠️ Service health after restart: {:?}", health),
    }

    // Clean stop
    sleep(Duration::from_secs(1)).await;
    println!("🛑 Stopping service...");
    manager.stop().await?;
    println!("✅ Service stopped");

    Ok(())
}

/// Example 5: Error Handling Patterns
/// 
/// This example shows how to handle various error conditions.
#[allow(dead_code)]
async fn example_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 5: Error Handling Patterns ===");

    let config = Config::default();
    let service = HttpServerServiceBuilder::new()
        .foreground_mode(false)
        .build();
    let mut manager = DefaultServiceManager::new(service, config, false);

    // Try to start the service
    match manager.start().await {
        Ok(_) => {
            println!("✅ Service started successfully");
            
            // Try to start again (should fail)
            match manager.start().await {
                Err(ServiceError::AlreadyRunning(name)) => {
                    println!("✅ Correctly detected already running service: {}", name);
                }
                Err(e) => {
                    println!("❌ Unexpected error: {}", e);
                }
                Ok(_) => {
                    println!("❌ Should have failed - service was already running");
                }
            }
            
            // Stop the service
            manager.stop().await?;
        }
        Err(e) => {
            println!("❌ Failed to start service: {}", e);
        }
    }

    // Try to stop a service that's not running
    match manager.stop().await {
        Ok(_) => {
            println!("✅ Stop operation completed (service was already stopped)");
        }
        Err(e) => {
            println!("❌ Unexpected error stopping service: {}", e);
        }
    }

    Ok(())
}

/// Example 6: Multiple Services Coordination
/// 
/// This example shows how to coordinate multiple services.
#[allow(dead_code)]
async fn example_multiple_services() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 6: Multiple Services Coordination ===");

    // Configure for multiple services
    let mut config = Config::default();
    config.cluster.enabled = true;
    config.cluster.grpc_port = 50051;
    config.cluster.http_port = 8080;
    config.load_balancer.enabled = true;
    config.load_balancer.port = 8090;

    // Create multiple service managers
    let http_service = HttpServerServiceBuilder::new()
        .foreground_mode(false)
        .build();
    let mut http_manager = DefaultServiceManager::new(http_service, config.clone(), false);

    let cluster_service = ClusterNodeServiceBuilder::new()
        .node_id("coordinator-node")
        .foreground_mode(false)
        .build()?;
    let mut cluster_manager = DefaultServiceManager::new(cluster_service, config.clone(), false);

    let lb_service = LoadBalancerServiceBuilder::new()
        .foreground_mode(false)
        .build();
    let mut lb_manager = DefaultServiceManager::new(lb_service, config, false);

    // Start all services
    println!("🚀 Starting all services...");
    
    // Start in dependency order (if any)
    match http_manager.start().await {
        Ok(_) => println!("✅ HTTP server started"),
        Err(e) => println!("❌ HTTP server failed: {}", e),
    }

    match cluster_manager.start().await {
        Ok(_) => println!("✅ Cluster node started"),
        Err(e) => println!("❌ Cluster node failed: {}", e),
    }

    match lb_manager.start().await {
        Ok(_) => println!("✅ Load balancer started"),
        Err(e) => println!("❌ Load balancer failed: {}", e),
    }

    // Monitor all services
    sleep(Duration::from_secs(2)).await;
    
    println!("\n📊 Service Status Check:");
    println!("HTTP Server: {:?}", http_manager.status());
    println!("Cluster Node: {:?}", cluster_manager.status());
    println!("Load Balancer: {:?}", lb_manager.status());

    // Stop all services in reverse order
    println!("\n🛑 Stopping all services...");
    
    let _ = lb_manager.stop().await;
    println!("✅ Load balancer stopped");
    
    let _ = cluster_manager.stop().await;
    println!("✅ Cluster node stopped");
    
    let _ = http_manager.stop().await;
    println!("✅ HTTP server stopped");

    Ok(())
}

/// Example 7: Foreground vs Background Mode
/// 
/// This example demonstrates the difference between foreground and background modes.
#[allow(dead_code)]
async fn example_foreground_vs_background() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 7: Foreground vs Background Mode ===");

    let config = Config::default();

    // Background mode example
    {
        println!("\n--- Background Mode ---");
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(false) // Background mode
            .build();
        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        manager.start().await?;
        println!("✅ Background service started");
        
        // In background mode, we can check status while it runs
        sleep(Duration::from_millis(500)).await;
        println!("📊 Status while running: {:?}", manager.status());
        
        manager.stop().await?;
        println!("✅ Background service stopped");
    }

    // Foreground mode example (simulated)
    {
        println!("\n--- Foreground Mode (Simulated) ---");
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(true) // Foreground mode
            .build();
        let manager = DefaultServiceManager::new(service, config, true);

        println!("ℹ️ In foreground mode, the service would block until stopped");
        println!("ℹ️ For demonstration, we're just showing the concept");
        println!("📊 Initial status: {:?}", manager.status());
        
        // In real foreground mode, you would:
        // manager.start().await?; // This would block until Ctrl+C or signal
    }

    Ok(())
}

/// Example 8: Custom Configuration and Validation
/// 
/// This example shows how to work with custom configurations.
#[allow(dead_code)]
async fn example_custom_configuration() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 8: Custom Configuration and Validation ===");

    // Create a custom configuration
    let mut config = Config::default();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 9090;
    config.cluster.enabled = true;
    config.cluster.node_id = "custom-node-123".to_string();
    config.cluster.grpc_port = 50052;

    // Validate configuration before using
    match HttpServerService::validate_config(&config) {
        Ok(_) => println!("✅ HTTP server configuration is valid"),
        Err(e) => println!("❌ HTTP server configuration error: {}", e),
    }

    match ClusterNodeService::validate_config(&config) {
        Ok(_) => println!("✅ Cluster node configuration is valid"),
        Err(e) => println!("❌ Cluster node configuration error: {}", e),
    }

    // Create service with explicit configuration
    let service = HttpServerServiceBuilder::new()
        .with_config(config.clone()) // Optional: set config at build time
        .foreground_mode(false)
        .build();

    let mut manager = DefaultServiceManager::new(service, config, false);

    // Use the service
    match manager.start().await {
        Ok(_) => {
            println!("✅ Service with custom configuration started");
            println!("🌐 Running on {}:{}", "127.0.0.1", 9090);
            
            sleep(Duration::from_secs(1)).await;
            manager.stop().await?;
            println!("✅ Service stopped");
        }
        Err(e) => {
            println!("❌ Service failed to start: {}", e);
        }
    }

    Ok(())
}

/// Migration Example: From Legacy Daemon to New Service
/// 
/// This example shows how to migrate from the old daemon system to the new one.
#[allow(dead_code)]
async fn example_migration_from_legacy() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Migration Example: Legacy to New Service ===");

    let config = Config::default();

    // Old way (deprecated - removed)
    println!("\n--- Legacy Approach (Removed) ---");
    println!("⚠️ Legacy DaemonService has been removed");
    println!("✅ Use the new service abstraction instead");

    // New way (recommended)
    println!("\n--- New Approach (Recommended) ---");
    {
        // Use the new service directly
        let service = HttpServerService::new(false); // background mode
        let mut manager = DefaultServiceManager::new(service, config, false);
        
        println!("✅ New service manager created directly");
        
        match manager.start().await {
            Ok(_) => {
                println!("✅ New service started successfully");
                
                // Show improved features
                println!("📊 Status: {:?}", manager.status());
                println!("💚 Health: {:?}", manager.health().await);
                println!("⏱️ Uptime: {:?}", manager.uptime());
                
                manager.stop().await?;
                println!("✅ New service stopped successfully"); 
            }
            Err(e) => {
                println!("❌ New service failed: {}", e);
            }
        }
    }

    Ok(())
}

/// Run all examples
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging for examples
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .init();

    println!("🎯 Voice-CLI Background Service Examples");
    println!("=========================================");

    // Run basic example
    if let Err(e) = example_http_server_basic().await {
        error!("Example 1 failed: {}", e);
    }

    // Add more examples as needed
    println!("\n✅ Examples completed");
    println!("\nTo try other examples, uncomment them in the main() function");
    println!("Available examples:");
    println!("  - example_cluster_node_auto_id()");
    println!("  - example_load_balancer_with_monitoring()");
    println!("  - example_service_restart_pattern()");
    println!("  - example_error_handling()");
    println!("  - example_multiple_services()");
    println!("  - example_foreground_vs_background()");
    println!("  - example_custom_configuration()");
    println!("  - example_migration_from_legacy()");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_service_lifecycle() {
        let config = Config::default();
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(false)
            .build();
        let mut manager = DefaultServiceManager::new(service, config, false);

        // Test basic lifecycle
        assert!(!manager.is_running());
        
        // Note: Actual start/stop would require server infrastructure
        // In integration tests, you would test the full lifecycle
        
        assert_eq!(manager.service_name(), "http-server");
    }

    #[tokio::test]
    async fn test_service_builder_patterns() {
        // Test various builder patterns
        let service1 = HttpServerServiceBuilder::new()
            .foreground_mode(true)
            .build();
        assert_eq!(service1.service_name(), "http-server");

        let service2 = ClusterNodeServiceBuilder::new()
            .node_id("test-node")
            .foreground_mode(false)
            .build()
            .unwrap();
        assert_eq!(service2.service_name(), "cluster-node");
        assert_eq!(service2.node_id(), "test-node");

        let service3 = LoadBalancerServiceBuilder::new()
            .foreground_mode(true)
            .build();
        assert_eq!(service3.service_name(), "load-balancer");
    }

    #[tokio::test]
    async fn test_configuration_validation() {
        let mut config = Config::default();
        
        // Valid configuration should pass
        assert!(HttpServerService::validate_config(&config).is_ok());
        
        // Invalid configuration should fail
        config.server.host = "".to_string();
        assert!(HttpServerService::validate_config(&config).is_err());
    }
}