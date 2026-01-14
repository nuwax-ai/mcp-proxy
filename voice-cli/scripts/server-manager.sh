#!/bin/bash

# Server Manager Script for voice-cli server
# Usage: ./server-manager.sh {start|stop|restart|status}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PID_FILE="${PROJECT_ROOT}/server.pid"
LOG_FILE="${PROJECT_ROOT}/logs/server.log"

# Ensure logs directory exists
mkdir -p "${PROJECT_ROOT}/logs"

# Function to start the server
start_server() {
    if [ -f "$PID_FILE" ]; then
        if kill -0 $(cat "$PID_FILE") 2>/dev/null; then
            echo "Server is already running (PID: $(cat $PID_FILE))"
            return 1
        else
            echo "Removing stale PID file"
            rm -f "$PID_FILE"
        fi
    fi

    echo "Starting voice-cli server..."
    VOICE_CLI_BIN="${SCRIPT_DIR}/voice-cli"
    if [ ! -x "$VOICE_CLI_BIN" ]; then
        echo "voice-cli binary not found in scripts directory: $VOICE_CLI_BIN"
        echo "Please place the compiled voice-cli binary in the same directory as this script."
        return 1
    fi

    # Start server in background and capture PID (use nohup to detach)
    nohup "$VOICE_CLI_BIN" server run >> "$LOG_FILE" 2>&1 &
    SERVER_PID=$!
    
    echo $SERVER_PID > "$PID_FILE"
    echo "Server started with PID: $SERVER_PID"
    echo "Logs: $LOG_FILE"
}

# Function to stop the server
stop_server() {
    if [ ! -f "$PID_FILE" ]; then
        echo "PID file not found. Server may not be running."
        return 1
    fi

    SERVER_PID=$(cat "$PID_FILE")
    
    if kill -0 $SERVER_PID 2>/dev/null; then
        echo "Stopping server (PID: $SERVER_PID)..."
        kill $SERVER_PID
        
        # Wait for process to terminate
        for i in {1..10}; do
            if ! kill -0 $SERVER_PID 2>/dev/null; then
                break
            fi
            sleep 1
        done
        
        if kill -0 $SERVER_PID 2>/dev/null; then
            echo "Server did not stop gracefully, forcing termination..."
            kill -9 $SERVER_PID
        fi
        
        rm -f "$PID_FILE"
        echo "Server stopped"
    else
        echo "Server not running (stale PID: $SERVER_PID)"
        rm -f "$PID_FILE"
    fi
}

# Function to restart the server
restart_server() {
    stop_server
    sleep 2
    start_server
}

# Function to check server status
status_server() {
    if [ -f "$PID_FILE" ]; then
        SERVER_PID=$(cat "$PID_FILE")
        if kill -0 $SERVER_PID 2>/dev/null; then
            echo "Server is running (PID: $SERVER_PID)"
            return 0
        else
            echo "Server is not running (stale PID: $SERVER_PID)"
            rm -f "$PID_FILE"
            return 1
        fi
    else
        echo "Server is not running"
        return 1
    fi
}

# Main script logic
case "$1" in
    start)
        start_server
        ;;
    stop)
        stop_server
        ;;
    restart)
        restart_server
        ;;
    status)
        status_server
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status}"
        exit 1
        ;;
esac

exit 0