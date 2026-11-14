package com.phloraxx.infraphone;

import android.app.Activity;
import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.view.Gravity;
import android.view.View;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

/** Shared, dependency-free visual language for the three Infra Phone screens. */
final class UiKit {
    static final int BG=0xff0c1214;
    static final int PANEL=0xff172023;
    static final int PANEL_2=0xff1d282b;
    static final int LINE=0xff2d3b3c;
    static final int INK=0xffeff4f1;
    static final int MUTED=0xff9eafab;
    static final int SUBTLE=0xff738681;
    static final int ACCENT=0xffaedfd0;
    static final int GOOD=0xff8fd0b4;
    static final int WARN=0xffe8bb84;

    private UiKit(){}
    static int dp(Context c,int size){return (int)(c.getResources().getDisplayMetrics().density*size+0.5f);}
    static GradientDrawable shape(Context c,int color,int radius,int strokeColor){
        GradientDrawable d=new GradientDrawable();
        d.setColor(color);d.setCornerRadius(dp(c,radius));
        if(strokeColor!=0)d.setStroke(dp(c,1),strokeColor);
        return d;
    }
    static RippleDrawable ripple(Context c,int fill,int radius,int border) {
        return new RippleDrawable(ColorStateList.valueOf(0x44ffffff),
            shape(c,fill,radius,border),null);
    }
    static LinearLayout screen(Activity activity){
        activity.getWindow().setStatusBarColor(BG);
        activity.getWindow().setNavigationBarColor(BG);
        if(android.os.Build.VERSION.SDK_INT>=29)
            activity.getWindow().setNavigationBarContrastEnforced(false);
        ScrollView scroll=new ScrollView(activity);
        scroll.setBackgroundColor(BG);
        scroll.setFillViewport(true);
        scroll.setClipToPadding(false);
        scroll.setVerticalScrollBarEnabled(false);
        LinearLayout root=new LinearLayout(activity);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(dp(activity,24),dp(activity,26),dp(activity,24),dp(activity,34));
        scroll.addView(root,new ScrollView.LayoutParams(-1,-2));
        activity.setContentView(scroll);
        return root;
    }
    static TextView text(Context c,String value,int sp,int color,boolean medium){
        TextView v=new TextView(c);
        v.setText(value);v.setTextSize(sp);v.setTextColor(color);
        v.setFontFeatureSettings("kern");
        v.setTypeface(Typeface.create(medium?"sans-serif-medium":"sans-serif",Typeface.NORMAL));
        v.setIncludeFontPadding(false);
        return v;
    }
    static TextView addText(LinearLayout parent,String value,int sp,int color,boolean medium){
        TextView t=text(parent.getContext(),value,sp,color,medium);
        parent.addView(t,new LinearLayout.LayoutParams(-1,-2));
        return t;
    }
    static void gap(LinearLayout parent,int size){
        View spacer=new View(parent.getContext());
        parent.addView(spacer,new LinearLayout.LayoutParams(1,dp(parent.getContext(),size)));
    }
    static LinearLayout card(LinearLayout parent){
        Context c=parent.getContext();
        LinearLayout inner=new LinearLayout(c);
        inner.setOrientation(LinearLayout.VERTICAL);
        inner.setPadding(dp(c,20),dp(c,22),dp(c,20),dp(c,22));
        inner.setBackground(shape(c,PANEL,24,LINE));
        LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(-1,-2);
        params.bottomMargin=dp(c,16);
        parent.addView(inner,params);
        return inner;
    }
    static TextView eyebrow(LinearLayout parent,String title){
        TextView t=addText(parent,title.toUpperCase(java.util.Locale.ROOT),11,SUBTLE,true);
        t.setLetterSpacing(.14f);
        return t;
    }
    static LinearLayout horizontal(Context c){
        LinearLayout row=new LinearLayout(c);row.setGravity(Gravity.CENTER_VERTICAL);
        row.setOrientation(LinearLayout.HORIZONTAL);return row;
    }
    static TextView button(LinearLayout parent,String label,boolean primary,Runnable action){
        Context c=parent.getContext();
        TextView t=text(c,label,15,primary?BG:INK,true);
        t.setGravity(Gravity.CENTER);
        t.setMinHeight(dp(c,52));
        t.setPadding(dp(c,16),dp(c,12),dp(c,16),dp(c,12));
        t.setBackground(ripple(c,primary?ACCENT:PANEL_2,16,primary?0:LINE));
        t.setClickable(true);t.setFocusable(true);
        LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(-1,-2);
        parent.addView(t,params);
        t.setOnClickListener(v->action.run());
        return t;
    }
    static TextView quietAction(LinearLayout parent,String label,Runnable action){
        Context c=parent.getContext();
        TextView t=text(c,label,14,MUTED,true);
        t.setGravity(Gravity.CENTER);t.setMinHeight(dp(c,48));
        t.setBackground(ripple(c,Color.TRANSPARENT,14,0));
        t.setClickable(true);t.setFocusable(true);
        parent.addView(t,new LinearLayout.LayoutParams(-1,-2));
        t.setOnClickListener(v->action.run());return t;
    }
    static void row(LinearLayout parent,int icon,String title,String detail,Runnable click){
        Context c=parent.getContext();
        LinearLayout whole=horizontal(c);
        whole.setPadding(dp(c,12),dp(c,14),dp(c,13),dp(c,14));
        whole.setBackground(ripple(c,PANEL,18,LINE));
        whole.setMinimumHeight(dp(c,76));
        whole.setClickable(true);whole.setFocusable(true);
        whole.setContentDescription(title+(detail.isEmpty()?"":", "+detail));
        ImageView image=new ImageView(c);
        image.setImageResource(icon);image.setColorFilter(ACCENT);
        image.setPadding(dp(c,8),dp(c,8),dp(c,8),dp(c,8));
        image.setBackground(shape(c,PANEL_2,12,0));
        whole.addView(image,new LinearLayout.LayoutParams(dp(c,40),dp(c,40)));
        LinearLayout middle=new LinearLayout(c);
        middle.setOrientation(LinearLayout.VERTICAL);
        LinearLayout.LayoutParams mp=new LinearLayout.LayoutParams(0,-2,1);
        mp.leftMargin=dp(c,14);mp.rightMargin=dp(c,10);
        whole.addView(middle,mp);
        addText(middle,title,15,INK,true);
        if(!detail.isEmpty()){
            gap(middle,4);
            addText(middle,detail,12,MUTED,false);
        }
        TextView arrow=text(c,"›",26,SUBTLE,false);
        whole.addView(arrow);
        LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(-1,-2);
        params.bottomMargin=dp(c,10);
        parent.addView(whole,params);
        whole.setOnClickListener(v->click.run());
    }
    static ImageView iconButton(LinearLayout parent,int icon,String description,Runnable click){
        Context c=parent.getContext();
        ImageView image=new ImageView(c);
        image.setImageResource(icon);image.setColorFilter(INK);
        image.setPadding(dp(c,13),dp(c,13),dp(c,13),dp(c,13));
        image.setBackground(ripple(c,PANEL,15,LINE));
        image.setContentDescription(description);
        image.setClickable(true);image.setFocusable(true);
        parent.addView(image,new LinearLayout.LayoutParams(dp(c,48),dp(c,48)));
        image.setOnClickListener(v->click.run());return image;
    }
    static EditText input(LinearLayout parent,String hint,String value){
        Context c=parent.getContext();
        EditText v=new EditText(c);
        v.setSingleLine(true);v.setTextSize(15);
        v.setTextColor(INK);v.setHintTextColor(SUBTLE);
        v.setHint(hint);v.setText(value);
        v.setSelectAllOnFocus(false);
        v.setPadding(dp(c,16),dp(c,13),dp(c,16),dp(c,13));
        v.setBackground(shape(c,PANEL_2,14,LINE));
        v.setMinHeight(dp(c,54));
        LinearLayout.LayoutParams p=new LinearLayout.LayoutParams(-1,-2);
        p.bottomMargin=dp(c,12);
        parent.addView(v,p);
        return v;
    }
    static LinearLayout header(LinearLayout root,String title,int icon,Runnable click){
        Context c=root.getContext();
        LinearLayout header=horizontal(c);
        LinearLayout.LayoutParams p=new LinearLayout.LayoutParams(-1,-2);
        p.bottomMargin=dp(c,42);
        root.addView(header,p);
        boolean home="infra.".equals(title);
        if(!home){
            iconButton(header,icon,"Back",click);
        }
        TextView logo=text(c,title,home?25:23,INK,true);
        logo.setLetterSpacing(-.035f);
        LinearLayout.LayoutParams textParams=new LinearLayout.LayoutParams(0,-2,1);
        if(!home)textParams.leftMargin=dp(c,15);
        header.addView(logo,textParams);
        if(home)iconButton(header,icon,"Settings",click);
        return header;
    }
    static void section(LinearLayout root,String name){
        eyebrow(root,name);gap(root,13);
    }
    static void note(LinearLayout root,String note){
        TextView t=addText(root,note,12,SUBTLE,false);t.setLineSpacing(dp(root.getContext(),4),1f);
    }
}
