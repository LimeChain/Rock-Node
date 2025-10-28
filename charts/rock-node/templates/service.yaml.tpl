apiVersion: v1
kind: Service
metadata:
  name: {{ include "rock-node.fullname" . }}
  namespace: {{ include "rock-node.namespace" . }}
  labels:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    helm.sh/chart: {{ include "rock-node.chart" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
    app.kubernetes.io/managed-by: {{ .Release.Service }}
  {{- with .Values.service.annotations }}
  annotations:
{{ toYaml . | indent 4 }}
  {{- end }}
spec:
  type: {{ .Values.service.type }}
  selector:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
  ports:
    - name: grpc
      port: {{ .Values.service.grpc.port }}
      targetPort: {{ .Values.service.grpc.targetPort }}
      protocol: TCP
      {{- if and (or (eq .Values.service.type "NodePort") (eq .Values.service.type "LoadBalancer")) .Values.service.grpc.nodePort }}
      nodePort: {{ .Values.service.grpc.nodePort }}
      {{- end }}
    {{- if .Values.service.metrics.enabled }}
    - name: metrics
      port: {{ .Values.service.metrics.port }}
      targetPort: {{ .Values.service.metrics.targetPort }}
      protocol: TCP
      {{- if and (or (eq .Values.service.type "NodePort") (eq .Values.service.type "LoadBalancer")) .Values.service.metrics.nodePort }}
      nodePort: {{ .Values.service.metrics.nodePort }}
      {{- end }}
    {{- end }}
