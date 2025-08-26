#!/bin/bash

# Load Balancer Manager Script for voice-cli load balancer
# Usage: ./lb-manager.sh {start|stop|restart|status}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PID_FILE="${PROJECT_ROOT}/lb.pid"
LOG_FILE="${PROJECT_ROOT}/logs/lb.log"

# Ensure logs directory exists
mkdir -p "${PROJECT_ROOT}/logs"

# Function to start the load balancer
start_lb() {
    if [ -f "$PID_FILE" ]; then
        if kill -0 $(cat "$PID_FILE") 2>/dev/null; then
            echo "Load balancer is already running (PID: $(cat $PID_FILE))"
            return 1
        else
            echo "Removing stale PID file"
            rm -f "$PID_FILE"
        fi
    fi

    echo "Starting voice-cli load balancer..."
    VOICE_CLI_BIN="${SCRIPT_DIR}/voice-cli"
    if [ ! -x "$VOICE_CLI_BIN" ]; then
        echo "voice-cli binary not found in scripts directory: $VOICE_CLI_BIN"
        echo "Please place the compiled voice-cli binary in the same directory as this script."
        return 1
    fi

    # Start load balancer in background and capture PID (use nohup to detach)
    nohup "$VOICE_CLI_BIN" lb run  >> "$LOG_FILE" 2>&1 &
    LB_PID=$!
    
    echo $LB_PID > "$PID_FILE"
    echo "Load balancer started with PID: $LB_PID"
    echo "Logs: $LOG_FILE"
}

# Function to stop the load balancer
stop_lb() {
    if [ ! -f "$PID_FILE" ]; then
        echo "PID file not found. Load balancer may not be running."
        return 1
    fi

    LB_PID=$(cat "$PID_FILE")
    
    if kill -0 $LB_PID 2>/dev/null; then
        echo "Stopping load balancer (PID: $LB_PID)..."
        kill $LB_PID
        
        # Wait for process to terminate
        for i in {1..10}; do
            if ! kill -0 $LB_PID 2>/dev/null; then
                break
            fi
            sleep 1
        done
        
        if kill -0 $LB_PID 2>/dev/null; then
            echo "Load balancer did not stop gracefully, forcing termination..."
            kill -9 $LB_PID
        fi
        
        rm -f "$PID_FILE"
        echo "Load balancer stopped"
    else
        echo "Load balancer not running (stale PID: $LB_PID)"
        rm -f "$PID_FILE"
    fi
}

# Function to restart the load balancer
restart_lb() {
    stop_lb
    sleep 2
    start_lb
}

# Function to check load balancer status
status_lb() {
    if [ -f "$PID_FILE" ]; then
        LB_PID=$(cat "$PID_FILE")
        if kill -0 $LB_PID 2>/dev/null; then
            echo "Load balancer is running (PID: $LB_PID)"
            return 0
        else
            echo "Load balancer is not running (stale PID: $LB_PID)"
            rm -f "$PID_FILE"
            return 1
        fi
    else
        echo "Load balancer is not running"
        return 1
    fi
}

# Main script logic
case "$1" in
    start)
        start_lb
        ;;
    stop)
        stop_lb
        ;;
    restart)
        restart_lb
        ;;
    status)
        status_lb
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status}"
        exit 1
        ;;
esac

exit 0