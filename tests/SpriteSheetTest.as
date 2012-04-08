package 
{
	import flash.display.Sprite;
	import flash.display.Bitmap;
	import flash.display.BitmapData;
	
	import backstage2d.display.Actor;
	import backstage2d.display.classic.BSSprite;
	import backstage2d.display.spritesheets.SpriteNode;
	import backstage2d.display.spritesheets.SpriteSheet;
	import flash.system.System;
	
	import flash.utils.getTimer;
	import flash.display.StageQuality;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public class SpriteSheetTest extends Sprite 
	{
		public function SpriteSheetTest() 
		{
			init();
		}
		private function init():void
		{
			stage.quality = "16X16LINEAR";
			//stage.quality = "high";
			var actors:Vector.<Actor> = new <Actor>[
				new BSSprite( b1 ),
				new BSSprite( b2 ),
				new BSSprite( b3 ),
				new BSSprite( b4 ),
				new BSSprite( b5 ),
				new BSSprite( b6 ),
				new BSSprite( b7 ),
				new BSSprite( b8 ),
				new BSSprite( b9 ),
				new BSSprite( b10 ),
				new BSSprite( b11 ),
				new BSSprite( b12 )
			];
			
			var mem:int = System.totalMemory;
			var startTime:Number = getTimer();
			
			var ss:SpriteSheet = new SpriteSheet( 512, 512 );
			
			trace("total time: " + (getTimer() - startTime));
			trace("used memory: " + (System.totalMemory - mem) );
			
			var bmd:BitmapData = ss.make( actors );
			addChild( new Bitmap( bmd, "always" ) );
			
		}
		
	}
}
