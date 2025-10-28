{{- if not .Values.config.existingConfigMap }}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "rock-node.fullname" . }}-config
  namespace: {{ include "rock-node.namespace" . }}
  labels:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    helm.sh/chart: {{ include "rock-node.chart" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
    app.kubernetes.io/managed-by: {{ .Release.Service }}
data:
  {{ .Values.config.fileName }}: |
{{- $.Values.config.content | indent 4 }}
{{- end }}
