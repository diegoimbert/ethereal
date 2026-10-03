// base-114: user-facing notices (engine `Event::Notification` + app confirmations) as kit
// toasts. Rendered in the shared toast stack hosted by the collab presence bar (ChatToasts).
export { NotificationToastItems, useEngineNotifications, useHasNotices } from "./NotificationToasts";
export { INFO_TOAST_MS, MAX_NOTICES, notify, onNotificationEvent, sentence, toastLook, useNotices, type Notice } from "./store";
