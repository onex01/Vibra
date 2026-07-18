#!/bin/bash
echo "Building Vibra..."
echo "Cleaning previous build..."
make clean
echo "Setting up build environment..."
make setup
echo "Running Vibra..."
make run