# S3A Verified Paper Registry — Cloud Run Deployment

`s3a-registry-server` is a stateless Cloud Run HTTP microservice backed by a GCS FUSE volume (`/mnt/gcs/verified_registry.s3a`), mirroring `s3a-lrs-server` (`https://34.74.163.88.sslip.io`).

## 1. Build & Push Container Image

```bash
gcloud builds submit \
  --tag us-east1-docker.pkg.dev/PROJECT_ID/s3a/s3a-registry-server:latest \
  -f deploy/registry-cloudrun/Dockerfile .
```

## 2. Deploy to Cloud Run with GCS FUSE Mount

```bash
gcloud run deploy s3a-registry-server \
  --image us-east1-docker.pkg.dev/PROJECT_ID/s3a/s3a-registry-server:latest \
  --region us-east1 \
  --platform managed \
  --allow-unauthenticated \
  --execution-environment gen2 \
  --add-volume name=gcs-registry-vol,type=cloud-storage,bucket=S3A_REGISTRY_BUCKET \
  --add-volume-mount volume=gcs-registry-vol,mount-path=/mnt/gcs \
  --set-env-vars S3A_REGISTRY_ARCHIVE_PATH=/mnt/gcs/verified_registry.s3a,S3A_LRS_URL=https://34.74.163.88.sslip.io \
  --set-secrets S3A_REGISTRY_TOKEN=s3a-registry-token:latest,S3A_LRS_TOKEN=s3a-lrs-token:latest
```

## 3. Schedule Periodic Status Watchdog (Cloud Scheduler)

```bash
gcloud scheduler jobs create http s3a-registry-watchdog-daily \
  --location us-east1 \
  --schedule "0 6 * * *" \
  --uri "https://YOUR_SERVICE_URL/api/v1/registry/watchdog/check" \
  --http-method POST \
  --headers "Authorization=Bearer YOUR_REGISTRY_TOKEN"
```
