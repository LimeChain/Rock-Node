{{- if and .Values.persistence.enabled (not .Values.persistence.existingClaim) }}
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: {{ include "rock-node.fullname" . }}-data
  namespace: {{ include "rock-node.namespace" . }}
  labels:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    helm.sh/chart: {{ include "rock-node.chart" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
    app.kubernetes.io/managed-by: {{ .Release.Service }}
  {{- with .Values.persistence.annotations }}
  annotations:
{{ toYaml . | indent 4 }}
  {{- end }}
spec:
  accessModes:
{{ toYaml .Values.persistence.accessModes | indent 4 }}
  resources:
    requests:
      storage: {{ .Values.persistence.size }}
  {{- if .Values.persistence.storageClassName }}
  storageClassName: {{ .Values.persistence.storageClassName }}
  {{- end }}
  {{- with .Values.persistence.selector }}
  selector:
{{ toYaml . | indent 4 }}
  {{- end }}
{{- end }}
