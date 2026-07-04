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
Image reference, defaulting the tag to the chart's appVersion.
*/}}
{{- define "loom.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
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
