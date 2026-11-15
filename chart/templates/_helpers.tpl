{{- define "teletext.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
app.kubernetes.io/part-of: teletext
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "teletext.ttxdSelector" -}}
app.kubernetes.io/name: ttxd
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "teletext.tokenEnv" -}}
TTXD_TOKEN_{{ . | upper | replace "-" "_" }}
{{- end }}
