# Implementation Plan

- [x] 1. Fix Module Organization and Import Conflicts

  - Remove ambiguous glob re-exports in models/mod.rs (request::_, worker::_)
  - Create explicit re-exports for conflicting types (TranscriptionRequest, AudioMetadata)
  - Fix unused imports in cluster.rs, task_scheduler.rs, and other modules
  - Remove unused variables and fix compiler warnings
  - _Requirements: 10.1, 10.2_

- [x] 2. Implement DashMap State Management

  - [x] 2.1 Create ClusterState struct with DashMap

    - Create new ClusterState struct using Arc<DashMap<String, ClusterNode>>
    - Add Arc<DashMap<String, TaskMetadata>> for task cache
    - Implement atomic operations for node and task updates
    - Replace current RwLock usage in task_scheduler.rs
    - _Requirements: 3.2, 10.3_

  - [x] 2.2 Update TaskScheduler to use ClusterState with DashMap
    - Modify SimpleTaskScheduler to use new ClusterState
    - Replace available_nodes_cache RwLock with DashMap operations
    - Update get_available_nodes_for_tasks to use atomic operations
    - _Requirements: 6.1, 6.2_

- [x] 3. Standardize Error Handling with Anyhow

  - [x] 3.1 Replace VoiceCliError with anyhow for application errors

    - Update main.rs and CLI handlers to use anyhow::Result
    - Keep ClusterError (thiserror) for library-level errors only
    - Add From<ClusterError> for anyhow::Error conversion
    - _Requirements: 4.1, 4.3, 10.4_

  - [x] 3.2 Implement error context extensions
    - Create ClusterResultExt trait for adding context
    - Add with_node_context and with_task_context helpers
    - Update cluster operations to use .context() for better error messages
    - _Requirements: 4.2, 4.3_

- [x] 4. Replace Process Spawning with Direct Async Services

  - [x] 4.1 Create ClusterServiceManager

    - Create new service manager that runs HTTP, gRPC, and heartbeat services concurrently
    - Use tokio::select! for concurrent service management
    - Add graceful shutdown coordination for all services
    - _Requirements: 2.1, 2.2, 2.3_

  - [x] 4.2 Replace Command::spawn in handle_cluster_start
    - Remove process spawning logic from cli/cluster.rs
    - Implement direct async service startup using ClusterServiceManager
    - Eliminate PID file management complexity
    - Update start_cluster_node_server to use real service implementations
    - _Requirements: 1.1, 1.2, 1.3_

- [x] 5. Enhance Configuration Management

  - [x] 5.1 Add environment variable overrides to Config

    - Extend Config::load_or_create to support environment variables
    - Support VOICE_CLI_HTTP_PORT and VOICE_CLI_GRPC_PORT overrides
    - Add comprehensive validation in Config::validate method
    - _Requirements: 3.1, 3.4, 9.3_

  - [x] 5.2 Implement configuration hot reload
    - Add configuration change detection mechanism
    - Implement hot reload without process restart
    - Update services to respond to configuration changes
    - _Requirements: 2.4, 3.1_

- [x] 6. Complete gRPC Service Implementation

  - [x] 6.1 Implement AudioClusterServiceImpl

    - Create missing AudioClusterServiceImpl struct
    - Implement all gRPC service methods (join_cluster, heartbeat, etc.)
    - Add proper error handling and logging for gRPC operations
    - _Requirements: 5.1, 5.2, 5.4_

  - [x] 6.2 Fix gRPC server integration
    - Fix missing AudioClusterServiceImpl in server.rs
    - Remove unused reflection feature references
    - Add connection pooling and retry logic for gRPC clients
    - _Requirements: 5.3, 5.5_

- [x] 7. Implement Cluster State Management Integration

  - [x] 7.1 Integrate ClusterState with existing services

    - Update MetadataStore to work with ClusterState
    - Modify TaskScheduler to use shared ClusterState
    - Add atomic node health monitoring
    - _Requirements: 7.1, 7.2, 7.4_

  - [x] 7.2 Add automatic service discovery
    - Implement node discovery on cluster startup
    - Add dynamic node addition and removal
    - Update cluster topology management
    - _Requirements: 7.3, 7.5_

- [x] 8. Implement Load Balancer Service

  - [x] 8.1 Create VoiceCliLoadBalancer

    - Implement load balancer with automatic node discovery
    - Add round-robin request routing to healthy nodes
    - Implement health check endpoints
    - _Requirements: 8.1, 8.2, 8.5_

  - [x] 8.2 Add dynamic routing management
    - Update routing table based on cluster changes
    - Implement circuit breaker for unhealthy nodes
    - Add retry logic with exponential backoff
    - _Requirements: 8.3, 8.4_

- [x] 9. Enhance Deployment and Management

  - [x] 9.1 Improve cluster initialization

    - Update cluster init to create proper directory structure
    - Add environment validation and dependency checking
    - Implement configuration file generation with defaults
    - _Requirements: 9.1, 9.3_

  - [x] 9.2 Add systemd service integration
    - Create systemd service file generation
    - Add support for custom service names and resource limits
    - Implement service status monitoring
    - _Requirements: 9.2, 9.4_

- [x] 10. Add Comprehensive Testing

  - [x] 10.1 Implement unit tests for core components

    - Add tests for ClusterState DashMap operations
    - Test TaskScheduler round-robin assignment logic
    - Test configuration validation and error handling
    - _Requirements: 10.5_

  - [x] 10.2 Create integration tests for cluster lifecycle
    - Test complete cluster initialization and node joining
    - Test task distribution and completion workflows
    - Test failure scenarios and recovery mechanisms
    - _Requirements: 10.5_

- [x] 11. Fix Code Quality Issues

  - [x] 11.1 Remove compiler warnings

    - Fix unused imports (DateTime, timeout, chrono::Utc, etc.)
    - Fix unused variables with proper prefixing
    - Remove ambiguous glob re-exports
    - Fix gRPC reflection feature references
    - _Requirements: 10.1, 10.2_

  - [x] 11.2 Add structured logging improvements
    - Implement consistent structured logging with node_id and service_type fields
    - Add performance metrics and monitoring integration
    - Update log levels for production use
    - _Requirements: 4.2, 4.4_
