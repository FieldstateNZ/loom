{{/*
Expand the name of the chart.
*/}}
{{- define "loom.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Fully qualified app name. Truncated at 63 chars for DNS/label limits.
*/}}
{{- define "loom.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/*
Chart name and version, as used by the standard chart label.
*/}}
{{- define "loom.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Common labels applied to every object.
*/}}
{{- define "loom.labels" -}}
helm.sh/chart: {{ include "loom.chart" . }}
{{ include "loom.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/*
Selector labels — the stable subset shared by all components of the release.
*/}}
{{- define "loom.selectorLabels" -}}
app.kubernetes.io/name: {{ include "loom.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
Selector labels for the loom-server workload specifically. Adds a component so
the server Service never accidentally selects the bundled PostgreSQL pods (which
share the name/instance labels).
*/}}
{{- define "loom.serverSelectorLabels" -}}
{{ include "loom.selectorLabels" . }}
app.kubernetes.io/component: server
{{- end -}}

{{/*
ServiceAccount name.
*/}}
{{- define "loom.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "loom.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Image reference. Precedence: image.digest > image.tag > Chart.appVersion.

  - image.digest set   -> "<repository>@sha256:<64 hex>" (immutable; tag ignored)
  - image.digest empty -> "<repository>:<tag or appVersion>" (legacy behaviour)

image.repository must be a bare repository (no tag, no digest) so the two
cannot be combined into an ambiguous reference.
*/}}
{{- define "loom.image" -}}
{{- $repo := toString .Values.image.repository -}}
{{- $lastSegment := regexFind "[^/]*$" $repo -}}
{{- if or (contains "@" $repo) (contains ":" $lastSegment) -}}
{{- fail (printf "image.repository %q must not contain a tag or digest; use image.tag or image.digest instead." $repo) -}}
{{- end -}}
{{- $digest := toString (default "" .Values.image.digest) -}}
{{- if $digest -}}
{{- if not (regexMatch "^sha256:[0-9a-f]{64}$" $digest) -}}
{{- fail (printf "image.digest %q is invalid: it must match ^sha256:[0-9a-f]{64}$ (sha256: prefix, 64 lowercase hex characters)." $digest) -}}
{{- end -}}
{{- printf "%s@%s" $repo $digest -}}
{{- else -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- printf "%s:%s" $repo $tag -}}
{{- end -}}
{{- end -}}

{{/*
Name of the Secret holding the bundled dev PostgreSQL credentials.
*/}}
{{- define "loom.postgresql.secretName" -}}
{{- printf "%s-postgresql" (include "loom.fullname" .) -}}
{{- end -}}

{{/*
Fail-fast guard: the app Secret must be named (unless the user supplies
everything via extraEnv, which we cannot introspect). Bundled Postgres only
covers DATABASE_URL, so encryption key + admin token always need a Secret.
*/}}
{{- define "loom.requireExistingSecret" -}}
{{- if not .Values.secrets.existingSecret -}}
{{- fail "secrets.existingSecret is required: create a Secret with LOOM_ENCRYPTION_KEY and LOOM_ROOT_ADMIN_TOKEN (and DATABASE_URL unless postgresql.enabled). See deploy/README.md." -}}
{{- end -}}
{{- end -}}

{{/*
Resolves where DATABASE_URL comes from. Exactly one source wins:

  1. postgresql.enabled=true      -> the bundled dev DB Secret (key DATABASE_URL)
  2. database.existingSecret set  -> that Secret, key database.urlKey
  3. otherwise (legacy)           -> secrets.existingSecret, key secrets.keys.databaseUrl

Emits "<secret name>|<key>". Conflicting configuration fails the render rather
than silently picking a winner.
*/}}
{{- define "loom.databaseUrlSource" -}}
{{- $db := .Values.database | default dict -}}
{{- $ext := toString (default "" $db.existingSecret) -}}
{{- if .Values.postgresql.enabled -}}
{{- if $ext -}}
{{- fail (printf "database.existingSecret (%q) is set but postgresql.enabled=true: the bundled PostgreSQL supplies DATABASE_URL. Choose one (disable postgresql or clear database.existingSecret)." $ext) -}}
{{- end -}}
{{- printf "%s|DATABASE_URL" (include "loom.postgresql.secretName" .) -}}
{{- else if $ext -}}
{{- $key := toString (default "" $db.urlKey) -}}
{{- if not $key -}}
{{- fail (printf "database.urlKey must be set when database.existingSecret (%q) is used." $ext) -}}
{{- end -}}
{{- printf "%s|%s" $ext $key -}}
{{- else -}}
{{- printf "%s|%s" .Values.secrets.existingSecret .Values.secrets.keys.databaseUrl -}}
{{- end -}}
{{- end -}}

{{/*
Whether a CA-only trust anchor for the database connection is configured.
*/}}
{{- define "loom.databaseCaEnabled" -}}
{{- $tls := (.Values.database | default dict).tls | default dict -}}
{{- if $tls.caSecret -}}true{{- end -}}
{{- end -}}

{{/*
The Secret key projected as the CA certificate. The chart mounts ONE key,
read-only, and rejects names matching the private-key naming guard. It does not
inspect Secret contents; the operator must select the known CA certificate key.
*/}}
{{- define "loom.databaseCaKey" -}}
{{- $tls := (.Values.database | default dict).tls | default dict -}}
{{- $key := toString (default "" $tls.caKey) -}}
{{- if not $key -}}
{{- fail (printf "database.tls.caKey must be set when database.tls.caSecret (%q) is used (default: ca.crt)." $tls.caSecret) -}}
{{- end -}}
{{- if regexMatch "(?i)(^|[._-])(key|private)([._-]|$)" $key -}}
{{- fail (printf "database.tls.caKey %q looks like a private key. Only a CA certificate may be mounted here (e.g. ca.crt); never mount key material." $key) -}}
{{- end -}}
{{- $key -}}
{{- end -}}

{{/* Fixed mount location of the CA certificate inside the container. */}}
{{- define "loom.databaseCaMountPath" -}}/etc/loom/db-ca{{- end -}}
{{- define "loom.databaseCaFile" -}}{{ include "loom.databaseCaMountPath" . }}/ca.crt{{- end -}}

{{/*
Guard: extraEnv may not duplicate or override any env var the chart renders for
the current values, and may not repeat a name. Kubernetes keeps the LAST entry
of a duplicated env name, so a stray extraEnv DATABASE_URL would silently
re-point the server; we refuse instead.
*/}}
{{- define "loom.validateExtraEnv" -}}
{{- $managed := list "LOOM_BIND_ADDR" "LOOM_RUN_MIGRATIONS" "DATABASE_URL" "LOOM_ENCRYPTION_KEY" "LOOM_ROOT_ADMIN_TOKEN" "LOOM_KEY_PEPPER" "PGSSLROOTCERT" "PGSSLCERT" "PGSSLKEY" -}}
{{- if .Values.config.batchPollIntervalSecs -}}{{- $managed = append $managed "LOOM_BATCH_POLL_INTERVAL_SECS" -}}{{- end -}}
{{- if .Values.config.rustLog -}}{{- $managed = append $managed "RUST_LOG" -}}{{- end -}}
{{- if .Values.otel.enabled -}}
{{- $managed = concat $managed (list "OTEL_EXPORTER_OTLP_ENDPOINT" "OTEL_EXPORTER_OTLP_PROTOCOL" "OTEL_SERVICE_NAME" "OTEL_RESOURCE_ATTRIBUTES" "LOOM_TELEMETRY_CAPTURE_CONTENT") -}}
{{- end -}}
{{- $seen := dict -}}
{{- range .Values.extraEnv -}}
{{- $name := toString (default "" .name) -}}
{{- if not $name -}}
{{- fail "extraEnv entries must have a non-empty name." -}}
{{- end -}}
{{- if has $name $managed -}}
{{- fail (printf "extraEnv must not set %q: it is managed by the chart and extraEnv cannot override chart-managed env (use the corresponding values instead)." $name) -}}
{{- end -}}
{{- if hasKey $seen $name -}}
{{- fail (printf "extraEnv lists %q more than once." $name) -}}
{{- end -}}
{{- $_ := set $seen $name true -}}
{{- end -}}
{{- end -}}
