# Installation

Running the published image: a container, Compose, a Kubernetes Deployment, or the Helm chart.

The image name and the current release tag are in the
[README](../README.md#installation). Every example here pulls the floating `latest` tag, which is
right for a deployment that should follow releases and wrong for one where an unattended restart
must not change the running version. Pin the release tag instead.

Images are multi-architecture manifest lists covering `linux/amd64` and `linux/arm64`, so Docker
picks the matching architecture and no platform flag is needed. Each tag is signed with
[cosign](https://docs.sigstore.dev/) under this repository's GitHub OIDC identity, so a pull can
be verified against the workflow that pushed it.

## Docker

```bash
docker run -d \
  --name cf-webhook-redirect \
  --restart unless-stopped \
  -p 8080:8080 \
  -v "$(pwd)/config.toml:/app/config.toml:ro" \
  -v "$(pwd)/secrets:/run/secrets:ro" \
  -e WEBHOOK_REDIRECT_SECRETS_DIR=/run/secrets \
  timmi6790/cloudflare-access-webhook-redirect
```

With `secrets/cloudflare__client_id` and `secrets/cloudflare__client_secret` holding the service
token, the credentials stay out of both the image and the process environment. Rotating either
file is picked up without a restart.

## Docker Compose

```yaml
services:
  webhook-redirect:
    image: timmi6790/cloudflare-access-webhook-redirect
    container_name: cf-webhook-redirect
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - ./config.toml:/app/config.toml:ro
    environment:
      - WEBHOOK_REDIRECT_SERVER__HOST=0.0.0.0
      - WEBHOOK_REDIRECT_TELEMETRY__LOG_LEVEL=info
      - WEBHOOK_REDIRECT_SECRETS_DIR=/run/secrets
    secrets:
      - cloudflare__client_id
      - cloudflare__client_secret

secrets:
  cloudflare__client_id:
    file: ./secrets/client-id
  cloudflare__client_secret:
    file: ./secrets/client-secret
```

Compose mounts each secret at `/run/secrets/<name>`, so the secret names are the configuration
keys: `cloudflare__client_id` fills `cloudflare.client_id`.

## Kubernetes

The proxy is built for a projected `Secret` volume. It follows the `..data` symlink the kubelet
rewrites on rotation and rebuilds itself when the mount changes, instead of serving with a
credential that has since been revoked.

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: cf-webhook-redirect
data:
  config.toml: |
    [webhook]
    target_base = "https://your-protected-service.com"

    [webhook.paths]
    "/webhook/.*" = ["ALL"]
    "/api/public/.*" = ["POST"]
---
apiVersion: v1
kind: Secret
metadata:
  name: cf-access-credentials
type: Opaque
stringData:
  # The file names are the configuration keys: `__` separates nesting levels.
  cloudflare__client_id: your-client-id
  cloudflare__client_secret: your-client-secret
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: cf-webhook-redirect
  namespace: default
spec:
  replicas: 2
  selector:
    matchLabels:
      app: cf-webhook-redirect
  template:
    metadata:
      labels:
        app: cf-webhook-redirect
    spec:
      containers:
        - name: cf-webhook-redirect
          image: timmi6790/cloudflare-access-webhook-redirect
          ports:
            - containerPort: 8080
          env:
            - name: WEBHOOK_REDIRECT_SERVER__HOST
              value: "0.0.0.0"
            - name: WEBHOOK_REDIRECT_CONFIG
              value: /etc/cf-webhook-redirect/config.toml
            - name: WEBHOOK_REDIRECT_SECRETS_DIR
              value: /run/secrets
          volumeMounts:
            - name: config
              mountPath: /etc/cf-webhook-redirect
              readOnly: true
            - name: credentials
              mountPath: /run/secrets
              readOnly: true
          livenessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
          readinessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
          resources:
            requests:
              cpu: 100m
              memory: 64Mi
            limits:
              cpu: 200m
              memory: 128Mi
      volumes:
        - name: config
          configMap:
            name: cf-webhook-redirect
        - name: credentials
          secret:
            secretName: cf-access-credentials
---
apiVersion: v1
kind: Service
metadata:
  name: cf-webhook-redirect
spec:
  selector:
    app: cf-webhook-redirect
  ports:
    - port: 80
      targetPort: 8080
  type: ClusterIP
```

The service-link variables a namespace injects are named after the release rather than after this
image, so the configuration contract cannot declare them in advance and a pod running it wants
`enableServiceLinks: false`.

## Helm

The chart is
[`cloudflare-access-webhook-redirect`](https://github.com/TimSchoenle/helm-charts/tree/main/charts/cloudflare-access-webhook-redirect)
in `TimSchoenle/helm-charts`. Its release job pins the image by digest, so a chart version names
one build rather than whatever a tag points at that day.
