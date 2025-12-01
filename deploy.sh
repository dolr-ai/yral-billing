#!/bin/bash

# Deployment script for yral-billing on Fly.io

set -e

echo "🚀 Deploying yral-billing to Fly.io..."

# Check if flyctl is installed
if ! command -v flyctl &> /dev/null; then
    echo "❌ flyctl is not installed. Please install it first:"
    echo "   curl -L https://fly.io/install.sh | sh"
    exit 1
fi

# Check if user is logged in to Fly
if ! flyctl auth whoami &> /dev/null; then
    echo "❌ Not logged in to Fly. Please run: flyctl auth login"
    exit 1
fi

# Create the app if it doesn't exist
if ! flyctl apps show yral-billing &> /dev/null; then
    echo "📝 Creating new Fly app: yral-billing"
    flyctl apps create yral-billing
fi

# Create the volume if it doesn't exist
if ! flyctl volumes list | grep -q "yral_billing_data"; then
    echo "💾 Creating persistent volume for database"
    flyctl volumes create yral_billing_data --region iad --size 1
fi

# Deploy the app
echo "🔨 Building and deploying..."
flyctl deploy

echo "✅ Deployment complete!"
echo "🔗 Your app is available at: https://yral-billing.fly.dev"

# Show logs
echo "📋 Recent logs:"
flyctl logs --app yral-billing