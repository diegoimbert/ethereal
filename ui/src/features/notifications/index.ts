// User-facing notices (engine `Event::Notification` + app confirmations) as kit
// toasts. Rendered in the shared toast stack hosted by the collab presence bar (ChatToasts).
export { useEngineNotifications, useHasNotices } from "./hooks";
export { NotificationLog, NotificationToastItems } from "./NotificationToasts";
export {
  ALERT_TOAST_MS,
  INFO_TOAST_MS,
  MAX_NOTICES,
  notify,
  onNotificationEvent,
  sentence,
  skipNotifications,
  toastLook,
  useNotices,
  type Notice,
  type NoticeAction,
  type NotificationEvent,
  type NotifyOptions,
} from "./store";
