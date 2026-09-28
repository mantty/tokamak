package com.tokamak.plugins.notifications

import android.content.ContentProvider
import android.content.ContentValues
import android.content.pm.PackageManager
import android.database.Cursor
import android.net.Uri
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import com.tokamak.runtime.tokamakHost
import java.util.UUID
import org.json.JSONObject

private const val APPLICATION_ID = "com.tokamak.notifications.fcm.application-id"
private const val PROJECT_ID = "com.tokamak.notifications.fcm.project-id"
private const val API_KEY = "com.tokamak.notifications.fcm.api-key"

/**
 * Starts Firebase from the Firebase project values in the app's manifest when the process
 * starts, before FCM delivers anything. FCM takes the sender ID from the application ID.
 */
class TokamakFirebaseInitializer : ContentProvider() {
    override fun onCreate(): Boolean {
        val context = context ?: return false
        val metadata =
            context.packageManager
                .getApplicationInfo(context.packageName, PackageManager.GET_META_DATA)
                .metaData ?: return true
        val applicationId = metadata.getString(APPLICATION_ID)
        val projectId = metadata.getString(PROJECT_ID)
        val apiKey = metadata.getString(API_KEY)
        if (applicationId == null || projectId == null || apiKey == null) return true
        val options =
            FirebaseOptions.Builder()
                .setApplicationId(applicationId)
                .setProjectId(projectId)
                .setApiKey(apiKey)
                .build()
        FirebaseApp.initializeApp(context, options)
        return true
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? = null

    override fun getType(uri: Uri): String? = null

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = 0
}

/** Receives FCM tokens and messages, including when FCM starts the app to deliver one. */
class TokamakPushService : FirebaseMessagingService() {
    private val plugin: TokamakNotificationsPlugin?
        get() = tokamakHost.plugin("notifications") as? TokamakNotificationsPlugin

    override fun onNewToken(token: String) {
        plugin?.onNewToken(token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        plugin?.onMessageReceived(message(message), message.notification != null)
    }

    /** The page's view of an FCM message: its data is the message's data map. */
    private fun message(message: RemoteMessage): JSONObject =
        JSONObject()
            .put("id", message.messageId ?: UUID.randomUUID().toString())
            .put("title", message.notification?.title ?: JSONObject.NULL)
            .put("body", message.notification?.body ?: JSONObject.NULL)
            .put("data", JSONObject(message.data as Map<*, *>))
}
