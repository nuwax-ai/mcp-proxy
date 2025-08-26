#!/bin/bash

# Cluster Manager Script for voice-cli cluster
# Usage: ./cluster-manager.sh {start|stop|restart|status}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
CONFIG_FILE="${PROJECT_ROOT}/cluster-config.yml"
PID_FILE="${PROJECT_ROOT}/cluster.pid"
LOG_FILE="${PROJECT_ROOT}/logs/cluster.log"

# Ensure logs directory exists
mkdir -p "${PROJECT_ROOT}/logs"

# Function to start the cluster
start_cluster() {
    if [ -f "$PID_FILE" ]; then
        if kill -0 $(cat "$PID_FILE") 2>/dev/null; then
            echo "Cluster is already running (PID: $(cat $PID_FILE))"
            return 1
        else
            echo "Removing stale PID file"
            rm -f "$PID_FILE"
        fi
    fi

    echo "Starting voice-cli cluster..."
    cd "$PROJECT_ROOT"
    
    # Start cluster in background and capture PID
    cargo run --bin voice-cli -- cluster run --config "$CONFIG_FILE" >> "$LOG_FILE" 2>&1 &
    CLUSTER_PID=$!
    
    echo $CLUSTER_PID > "$PID_FILE"
    echo "Cluster started with PID: $CLUSTER_PID"
    echo "Logs: $LOG_FILE"
}

# Function to stop the cluster
stop_cluster() {
    if [ ! -f "$PID_FILE" ]; then
        echo "PID file not found. Cluster may not be running."
        return 1
    fi

    CLUSTER_PID=$(cat "$PID_FILE")
    
    if kill -0 $CLUSTER_PID 2>/dev/null; then
        echo "Stopping cluster (PID: $CLUSTER_PID)..."
        kill $CLUSTER_PID
        
        # Wait for process to terminate
        for i in {1..10}; do
            if ! kill -0 $CLUSTER_PID 2>/dev/null; then
                break
            fi
            sleep 1
        done
        
        if kill -0 $CLUSTER_PID 2>/dev/null; then
            echo "Cluster did not stop gracefully, forcing termination..."
            kill -9 $CLUSTER_PID
        fi
        
        rm -f "$PID_FILE"
        echo "Cluster stopped"
    else
        echo "Cluster not running (stale PID: $CLUSTER_PID)"
        rm -f "$PID_FILE"
    fi
}

# Function to restart the cluster
restart_cluster() {
    stop_cluster
    sleep 2
    start_cluster
}

# Function to check cluster status
status_cluster() {
    if [ -f "$PID_FILE" ]; then
        CLUSTER_PID=$(cat "$PID_FILE")
        if kill -0 $CLUSTER_PID 2>/dev/null; then
            echo "Cluster is running (PID: $CLUSTER_PID)"
            return 0
        else
            echo "Cluster is not running (stale PID: $CLUSTER_PID)"
            rm -f "$PID_FILE"
            return 1
        fi
    else
        echo "Cluster is not running"
        return 1
    fi
}

# Main script logic
case "$1" in
    start)
        start_cluster
        ;;
    stop)
        stop_cluster
        ;;
    restart)
        restart_cluster
        ;;
    status)
        status_cluster
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status}"
        exit 1
        ;;
esac

exit 0